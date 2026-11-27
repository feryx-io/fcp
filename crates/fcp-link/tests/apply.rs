//! Configuration at the site (`docs/HYBRID_CONTROL_PLANE.md` phase 3): the
//! link keeps the site's Postgres a copy of the tenant's slice, so a user
//! made in the cloud registers at the site's registrar within seconds
//! with the password set in the cloud; a password changed in the cloud
//! is the one the site challenges with, and the old one is refused; the
//! site's own admin refuses a write to a synced object, naming the cloud,
//! and still reads and acts; and the cloud shows where the site's sync
//! stands. Gated on `FCP_TEST_DATABASE_URL`; skipped with a notice when
//! unset.

mod common;

use common::{admin, cloud_with_site_at, until};
use fcp_testkit::smoke::{free_port, http_request, loopback, toml_path};
use fcp_testkit::{bins, Phone, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

/// The realm the cloud's API computes digest credentials for, and the
/// site registrar's default.
const REALM: &str = "fcp.local";
const SITE_ADMIN_TOKEN: &str = "site-admin-token-change-me-32-chars";

/// Register until the registrar answers `want`, or give up.
async fn registers_with(what: &str, phone: &Phone, want: bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let code = phone.register().await.unwrap();
        if (code == 200) == want {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: the registrar kept answering {code}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_made_in_the_cloud_registers_at_the_site_with_the_password_the_cloud_set() {
    // The site's own database, and the cloud with a site whose link
    // keeps it.
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the apply test");
        return;
    };
    let cloud = cloud_with_site_at(Some(site_db.url()))
        .await
        .expect("the database was there a moment ago");

    // The site's registrar, on the site's database, in the tenant.
    let (sip, registrar_http, site_admin_http) = (free_port(), free_port(), free_port());
    let registrar_config = format!(
        r#"[service]
node_id = "acme-hq-registrar"
default_tenant_id = "{tenant}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{registrar_http}"

[logging]
level = "info"
format = "compact"
"#,
        tenant = cloud.tenant_id,
        db = site_db.url(),
    );
    let registrar = Service::start_with(
        "fcp-registrar",
        bins::path("fcp-registrar", "fcp-registrar"),
        &registrar_config,
        &loopback(registrar_http, "/health"),
        &[],
    )
    .await;
    let sip_addr: SocketAddr = format!("127.0.0.1:{sip}").parse().unwrap();

    // The site's admin: a site of the cloud, so synced objects are the
    // cloud's to change.
    let site_admin_config = format!(
        r#"prompts_dir = "{dir}/site-prompts"

[service]
node_id = "acme-hq-admin"

[api]
bind_address = "127.0.0.1:{site_admin_http}"
auth_enabled = true
auth_token = "{SITE_ADMIN_TOKEN}"
managed_by = "http://127.0.0.1:{cloud_http}"

[storage]
database_url = "{db}"

[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(cloud.dir.path()),
        cloud_http = cloud.http,
        db = site_db.url(),
    );
    let site_admin = Service::start_with(
        "fcp-admin (site)",
        bins::path("fcp-admin", "fcp-admin"),
        &site_admin_config,
        &loopback(site_admin_http, "/api/v1/health"),
        &[],
    )
    .await;

    // The snapshot lands: the link's status shows a cursor, and the
    // cloud shows the site caught up.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let status = loop {
        let (_, text) = http_request("GET", &loopback(cloud.link_health, "/status"), &[], None)
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_str(&text).unwrap();
        if status["sync"]["cursor"].as_i64().unwrap_or(0) > 0 {
            break status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the link never applied the snapshot: {status}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert!(status["sync"]["error"].is_null(), "{status}");
    assert!(
        status["sync_applied"].as_u64().unwrap() >= 1,
        "the tenant itself: {status}"
    );

    // Nothing changes for longer than a whole poll: the feed's long poll
    // answers empty, and the sync reports no error (found by the nightly's
    // site stack, where the link's request timeout was shorter than the
    // wait).
    tokio::time::sleep(Duration::from_secs(9)).await;
    let (_, text) = http_request("GET", &loopback(cloud.link_health, "/status"), &[], None)
        .await
        .unwrap();
    let idle: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        idle["sync"]["error"].is_null(),
        "an idle feed is no error: {idle}"
    );
    let shown = until(
        "the cloud never saw the site caught up",
        cloud.http,
        &cloud.site_path(),
        Some(&cloud.tenant_id),
        |s| s["sync_behind"] == 0 && s["sync_cursor"].as_i64().is_some_and(|c| c > 0),
    )
    .await;
    assert!(shown["sync_applied_at"].as_str().is_some(), "{shown}");

    // A user made in the cloud registers at the site within seconds.
    let (code, made) = cloud
        .admin_call(
            "POST",
            "/api/v1/users",
            true,
            Some(
                &serde_json::json!({
                    "username": "alice",
                    "password": "Alice-at-the-site-2026!",
                    "display_name": "Alice",
                    "tenant_id": cloud.tenant_id,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    let alice = Phone::new("alice", "Alice-at-the-site-2026!", REALM, sip_addr)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    registers_with("alice never registered at the site", &alice, true).await;
    eprintln!(
        "alice registered at the site {:?} after her creation",
        started.elapsed()
    );
    let wrong = Phone::new("alice", "not-her-password", REALM, sip_addr)
        .await
        .unwrap();
    assert_ne!(wrong.register().await.unwrap(), 200, "a wrong password");

    // A password changed in the cloud is the one the site challenges
    // with; the old one is refused.
    let (code, changed) = cloud
        .admin_call(
            "PUT",
            "/api/v1/users/alice/password",
            true,
            Some(&serde_json::json!({ "password": "Alice-renewed-2026!" }).to_string()),
        )
        .await;
    assert!(code == 200 || code == 204, "{changed}");
    registers_with(
        "the old password was still taken at the site",
        &alice,
        false,
    )
    .await;
    let renewed = Phone::new("alice", "Alice-renewed-2026!", REALM, sip_addr)
        .await
        .unwrap();
    registers_with("the new password never reached the site", &renewed, true).await;

    // The site's admin reads the synced user and refuses to change her,
    // naming the cloud; a runtime action goes through.
    let (code, listed) = admin(
        site_admin_http,
        "GET",
        "/api/v1/users",
        Some(&cloud.tenant_id),
        None,
    )
    .await;
    // The helper carries the cloud's token; the site's admin has its own.
    let _ = (code, listed);
    let site_bearer = format!("Bearer {SITE_ADMIN_TOKEN}");
    let site_headers = [
        ("Authorization", site_bearer.as_str()),
        ("Content-Type", "application/json"),
        ("x-fcp-tenant-id", cloud.tenant_id.as_str()),
    ];
    let (code, text) = http_request(
        "GET",
        &loopback(site_admin_http, "/api/v1/users"),
        &site_headers,
        None,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "{text}");
    assert!(
        text.contains("\"alice\""),
        "the synced user is read at the site: {text}"
    );
    let (code, text) = http_request(
        "POST",
        &loopback(site_admin_http, "/api/v1/users"),
        &site_headers,
        Some(
            &serde_json::json!({
                "username": "bob",
                "password": "Bob-at-the-site-2026!",
                "tenant_id": cloud.tenant_id,
            })
            .to_string(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(code, 403, "{text}");
    assert!(
        text.contains(&format!("http://127.0.0.1:{}", cloud.http)),
        "the refusal names the cloud: {text}"
    );
    let (code, text) = http_request(
        "PUT",
        &loopback(site_admin_http, "/api/v1/users/alice/password"),
        &site_headers,
        Some(&serde_json::json!({ "password": "Nope-2026!" }).to_string()),
    )
    .await
    .unwrap();
    assert_eq!(code, 403, "{text}");
    let (code, text) = http_request(
        "GET",
        &loopback(site_admin_http, "/api/v1/registrations"),
        &site_headers,
        None,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "the site's own runtime is readable: {text}");
    assert!(
        text.contains("alice"),
        "alice's registration at the site: {text}"
    );

    // The tenant's identity provider reaches the site with its client
    // secret, so the site signs its people in itself
    // (docs/IDENTITY_AND_DEVICES.md Q11); the site's admin may read it,
    // not change it.
    let (code, made) = cloud
        .admin_call(
            "POST",
            "/api/v1/identity-providers",
            true,
            Some(
                &serde_json::json!({
                    "name": "Acme SSO",
                    "settings": {"issuer": "https://login.example.com/acme/v2.0", "client_id": "fcp"},
                    "client_secret": "the-client-secret",
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let synced = loop {
        let (code, text) = http_request(
            "GET",
            &loopback(site_admin_http, "/api/v1/identity-providers"),
            &site_headers,
            None,
        )
        .await
        .unwrap();
        assert_eq!(code, 200, "{text}");
        let listed: serde_json::Value = serde_json::from_str(&text).unwrap();
        if let Some(p) = listed["data"].as_array().and_then(|d| d.first()) {
            break p.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the provider never reached the site: {text}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert_eq!(synced["name"], "Acme SSO", "{synced}");
    assert_eq!(synced["has_client_secret"], true, "{synced}");
    let (code, text) = http_request(
        "DELETE",
        &loopback(
            site_admin_http,
            &format!(
                "/api/v1/identity-providers/{}",
                synced["id"].as_str().unwrap()
            ),
        ),
        &site_headers,
        None,
    )
    .await
    .unwrap();
    assert_eq!(code, 403, "a synced provider is the cloud's: {text}");

    // The tenant's own TTS account and its speech reach the site, the key
    // with them (docs/TENANT_SPEECH.md S5); the site's admin may read them,
    // not change them, and may still test the account.
    let (code, account) = cloud
        .admin_call(
            "POST",
            "/api/v1/speech/accounts",
            true,
            Some(
                &serde_json::json!({"name": "Acme voices", "kind": "openai", "secret": "sk-acme"})
                    .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{account}");
    let provider = account["provider"].as_str().unwrap().to_string();
    let (code, chose) = cloud
        .admin_call(
            "PUT",
            "/api/v1/speech/settings",
            true,
            Some(&serde_json::json!({"provider": provider, "language": "en-GB"}).to_string()),
        )
        .await;
    assert_eq!(code, 200, "{chose}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (code, text) = http_request(
            "GET",
            &loopback(site_admin_http, "/api/v1/speech/settings"),
            &site_headers,
            None,
        )
        .await
        .unwrap();
        assert_eq!(code, 200, "{text}");
        let settings: serde_json::Value = serde_json::from_str(&text).unwrap();
        if settings["provider"] == provider.as_str() {
            assert_eq!(settings["language"], "en-GB", "{text}");
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the tenant's speech never reached the site: {text}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let (code, text) = http_request(
        "GET",
        &loopback(site_admin_http, "/api/v1/speech/accounts"),
        &site_headers,
        None,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "{text}");
    let listed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(listed["data"][0]["name"], "Acme voices", "{text}");
    assert_eq!(
        listed["data"][0]["has_key"], true,
        "the key came with it: {text}"
    );
    assert!(!text.contains("sk-acme"), "{text}");
    let account_id = listed["data"][0]["id"].as_str().unwrap().to_string();
    for (method, path, body) in [
        (
            "PUT",
            "/api/v1/speech/settings".to_string(),
            Some(r#"{"language":"fr-FR"}"#),
        ),
        (
            "DELETE",
            format!("/api/v1/speech/accounts/{account_id}"),
            None,
        ),
    ] {
        let (code, text) = http_request(
            method,
            &loopback(site_admin_http, &path),
            &site_headers,
            body,
        )
        .await
        .unwrap();
        assert_eq!(code, 403, "{method} {path} is the cloud's: {text}");
    }
    let (code, text) = http_request(
        "POST",
        &loopback(
            site_admin_http,
            &format!("/api/v1/speech/accounts/{account_id}/test"),
        ),
        &site_headers,
        None,
    )
    .await
    .unwrap();
    assert_ne!(code, 403, "testing an account is the site's own: {text}");

    // What only the site holds of Alice: a declared location and a queue
    // seat, which no feed carries.
    let site_pool = sqlx::PgPool::connect(site_db.url()).await.unwrap();
    let alice_id: String = sqlx::query_scalar("SELECT id FROM users WHERE username = 'alice'")
        .fetch_one(&site_pool)
        .await
        .unwrap();
    // A group made in the cloud, with Alice in it, reaches the site — its
    // role is hers there too — and the site's admin may not change it.
    let (code, group) = cloud
        .admin_call(
            "POST",
            "/api/v1/groups",
            true,
            Some(&serde_json::json!({"name": "Front desk", "role": "user_admin"}).to_string()),
        )
        .await;
    assert_eq!(code, 201, "{group}");
    let group_id = group["id"].as_str().unwrap().to_string();
    let (code, added) = cloud
        .admin_call(
            "POST",
            &format!("/api/v1/groups/{group_id}/members"),
            true,
            Some(&serde_json::json!({"username": "alice"}).to_string()),
        )
        .await;
    assert_eq!(code, 204, "{added}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let role: Option<String> = sqlx::query_scalar(
            "SELECT g.role FROM groups g JOIN group_members m ON m.group_id = g.id
             WHERE m.user_id = $1",
        )
        .bind(&alice_id)
        .fetch_optional(&site_pool)
        .await
        .unwrap()
        .flatten();
        if role.as_deref() == Some("user_admin") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the group never reached the site"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let (code, text) = http_request(
        "POST",
        &loopback(site_admin_http, "/api/v1/groups"),
        &site_headers,
        Some(&serde_json::json!({"name": "Local"}).to_string()),
    )
    .await
    .unwrap();
    assert_eq!(code, 403, "groups are the cloud's: {text}");

    for statement in [
        "INSERT INTO declared_locations (tenant_id, user_id, address, expires_at)
         SELECT tenant_id, id, '{}'::jsonb, now() + interval '1 day' FROM users WHERE id = $1",
        "INSERT INTO queue_agents (tenant_id, agent_id, extension)
         SELECT tenant_id, username, username FROM users WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(&alice_id)
            .execute(&site_pool)
            .await
            .unwrap();
    }

    // A user renamed in the cloud is renamed at the site (the row is
    // matched by its id), and her phone, keeping its SIP name, registers
    // there as before.
    let (code, renamed) = cloud
        .admin_call(
            "PUT",
            "/api/v1/users/alice",
            true,
            Some(&serde_json::json!({"username": "alice.w"}).to_string()),
        )
        .await;
    assert_eq!(code, 200, "{renamed}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let at_site: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT username, sip_username FROM users WHERE id = $1")
                .bind(&alice_id)
                .fetch_optional(&site_pool)
                .await
                .unwrap();
        if at_site == Some(("alice.w".to_string(), Some("alice".to_string()))) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the rename never reached the site: {at_site:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    registers_with("alice's phone after her rename", &renewed, true).await;

    // A user deleted in the cloud is gone at the site.
    let (code, gone) = cloud
        .admin_call("DELETE", "/api/v1/users/alice.w", true, None)
        .await;
    assert!(code == 200 || code == 204, "{gone}");
    // Deleted, then purged without waiting for the grace period.
    let (code, purged) = cloud
        .admin_call("POST", "/api/v1/users/alice.w/purge", true, None)
        .await;
    assert_eq!(code, 200, "{purged}");
    registers_with("alice still registered after her deletion", &renewed, false).await;
    // Purged at the site too: what the site alone held went with her.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1")
            .bind(&alice_id)
            .fetch_one(&site_pool)
            .await
            .unwrap();
        if left == 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "alice was never purged at the site"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for (what, sql, by) in [
        (
            "declared location",
            "SELECT count(*) FROM declared_locations WHERE user_id = $1",
            alice_id.as_str(),
        ),
        (
            "queue seat",
            "SELECT count(*) FROM queue_agents WHERE agent_id = $1",
            "alice",
        ),
        (
            "group membership",
            "SELECT count(*) FROM group_members WHERE user_id = $1",
            alice_id.as_str(),
        ),
    ] {
        let left: i64 = sqlx::query_scalar(sql)
            .bind(by)
            .fetch_one(&site_pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "alice's {what} outlived her at the site");
    }
    site_pool.close().await;

    let log = registrar.stop().await;
    assert!(!log.contains("panicked"), "the registrar panicked:\n{log}");
    let log = site_admin.stop().await;
    assert!(!log.contains("panicked"), "the site admin panicked:\n{log}");
    cloud.finish().await;
    site_db.drop_db().await;
}
