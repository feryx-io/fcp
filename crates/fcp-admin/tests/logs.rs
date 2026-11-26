//! The services' logs from the admin (`docs/LOG_VIEWER.md` L3), against a
//! running admin, Postgres and Redis: lines kept by two services are listed
//! newest first and filtered by level, text, service and tenant, paged by
//! cursor and exported as JSON Lines; a tenant's administrator sees only
//! its own tenant's lines, whatever it asks for, and cannot change a
//! service's level; a platform administrator sets one for an hour and
//! takes it back. Skipped without `FCP_TEST_DATABASE_URL` and
//! `FCP_TEST_REDIS_URL`.

use fcp_cluster::logging::{self, LogRecord};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "logs-static-token-0123456789abcdef";
const OURS: &str = "00000000-0000-0000-0000-000000000000";
const THEIRS: &str = "11111111-1111-1111-1111-111111111111";

async fn ship(redis: &fcp_storage_redis::RedisStorage, service: &str, lines: &[LogRecord]) {
    let mut conn = redis.connection();
    for line in lines {
        let _: String = redis::cmd("XADD")
            .arg(redis.key(&logging::stream_key(service)))
            .arg("*")
            .arg("r")
            .arg(serde_json::to_string(line).unwrap())
            .query_async(&mut conn)
            .await
            .unwrap();
        // A stream's ids are by the millisecond: one apiece, so newest
        // first is the order written.
        tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    }
}

fn line(level: &str, tenant: Option<&str>, message: &str) -> LogRecord {
    LogRecord {
        ts: 0,
        level: level.into(),
        target: "fcp_sip::handler".into(),
        message: message.into(),
        fields: BTreeMap::new(),
        tenant_id: tenant.map(String::from),
        call_id: None,
    }
}

async fn send(
    http: u16,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (u16, String) {
    let client = reqwest::Client::new();
    let mut req = client
        .request(method.parse().unwrap(), loopback(http, path))
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.unwrap();
    (res.status().as_u16(), res.text().await.unwrap_or_default())
}

async fn messages(http: u16, query: &str, token: &str) -> (Vec<String>, Value) {
    let (code, text) = send(http, "GET", &format!("/api/v1/logs{query}"), token, None).await;
    assert_eq!(code, 200, "{query}: {text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    let list = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["message"].as_str().unwrap().to_string())
        // The admin ships its own lines too; these are the test's.
        .filter(|m| {
            ["ours:", "theirs:", "platform:"]
                .iter()
                .any(|p| m.starts_with(p))
        })
        .collect();
    (list, body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn logs_are_read_filtered_and_held_to_their_tenant() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the logs test");
        return;
    };
    let Some(redis_url) = private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the logs test");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "logs-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{TOKEN}"

[storage]
database_url = "{db}"

[cluster]
redis_url = "{redis_url}"

[logging]
level = "info"
format = "compact"
"#,
        db = db.url(),
        dir = smoke::toml_path(dir.path()),
    );
    let admin = Service::start("admin", BIN, &config, &loopback(http, "/api/v1/health")).await;
    let redis = fcp_storage_redis::RedisStorage::new(fcp_storage_redis::RedisConfig {
        url: redis_url.clone(),
        ..Default::default()
    })
    .await
    .unwrap();

    ship(
        &redis,
        "cm-test-1",
        &[
            line("INFO", Some(OURS), "ours: alice registered"),
            line("WARN", Some(THEIRS), "theirs: bob busy"),
            line("INFO", None, "platform: node started"),
        ],
    )
    .await;
    ship(
        &redis,
        "sbc-test-1",
        &[
            line("ERROR", Some(OURS), "ours: trunk refused the call"),
            line("DEBUG", Some(OURS), "ours: a detail"),
        ],
    )
    .await;

    // The platform: everything, newest first.
    let (all, _) = messages(http, "?q=%3A%20", TOKEN).await;
    for m in [
        "ours: alice registered",
        "theirs: bob busy",
        "platform: node started",
        "ours: trunk refused the call",
        "ours: a detail",
    ] {
        assert!(all.contains(&m.to_string()), "{m}: {all:?}");
    }
    assert_eq!(all[0], "ours: a detail", "newest first: {all:?}");
    let (warned, _) = messages(http, "?level=warn&q=%3A%20", TOKEN).await;
    assert_eq!(
        warned,
        vec!["ours: trunk refused the call", "theirs: bob busy"]
    );
    let (platform, _) = messages(http, "?tenant=none&q=platform", TOKEN).await;
    assert_eq!(platform, vec!["platform: node started"]);
    let (theirs, _) = messages(http, &format!("?tenant={THEIRS}"), TOKEN).await;
    assert_eq!(theirs, vec!["theirs: bob busy"]);
    let (one_service, _) = messages(http, "?service=sbc-test-1", TOKEN).await;
    assert_eq!(one_service.len(), 2, "{one_service:?}");
    let (busy, _) = messages(http, "?q=BUSY", TOKEN).await;
    assert_eq!(busy, vec!["theirs: bob busy"]);

    // Paged by cursor.
    let (first, body) = messages(http, "?q=ours&limit=2", TOKEN).await;
    assert_eq!(first.len(), 2);
    let cursor = body["next_cursor"]
        .as_str()
        .expect("a next page")
        .to_string();
    let (second, body) = messages(http, &format!("?q=ours&limit=2&cursor={cursor}"), TOKEN).await;
    assert_eq!(second, vec!["ours: alice registered"], "{first:?}");
    assert!(body["next_cursor"].is_null());

    // A tenant's administrator: their own lines, whatever they ask for.
    let made = send(
        http,
        "POST",
        "/api/v1/users",
        TOKEN,
        Some(json!({
            "username": "ta",
            "password": "Str0ng-Passw0rd!",
            "role": "tenant_admin",
            "email": "ta@logs.test",
        })),
    )
    .await;
    assert!(made.0 < 300, "{made:?}");
    let (_, signed_in) = send(
        http,
        "POST",
        "/api/v1/auth/login",
        "none",
        Some(json!({"username": "ta", "password": "Str0ng-Passw0rd!"})),
    )
    .await;
    let signed_in: Value = serde_json::from_str(&signed_in).unwrap();
    let ta = signed_in["token"].as_str().expect("signed in").to_string();
    for query in ["", "?tenant=none", &format!("?tenant={THEIRS}")] {
        let (seen, _) = messages(http, query, &ta).await;
        assert!(!seen.is_empty(), "{query}");
        assert!(
            seen.iter().all(|m| m.starts_with("ours:")),
            "{query}: {seen:?}"
        );
    }
    let (code, export) = send(http, "GET", "/api/v1/logs/export", &ta, None).await;
    assert_eq!(code, 200, "{export}");
    let exported: Vec<Value> = export
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Every line is the tenant's own: the test's three, and the admin's
    // about this tenant (its sign-in).
    assert!(exported.iter().all(|l| l["tenant_id"] == OURS), "{export}");
    let seeded = exported
        .iter()
        .filter(|l| {
            l["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("ours:"))
        })
        .count();
    assert_eq!(seeded, 3, "{export}");
    assert!(
        export.contains("Login successful"),
        "the admin's own line about the tenant"
    );
    let (code, _) = send(
        http,
        "PUT",
        "/api/v1/logs/levels/cm-test-1",
        &ta,
        Some(json!({"level": "debug"})),
    )
    .await;
    assert_eq!(code, 403);
    let (code, _) = send(http, "GET", "/api/v1/logs/levels", &ta, None).await;
    assert_eq!(code, 403);

    // The platform sets a service's level for an hour, and takes it back.
    let (code, set) = send(
        http,
        "PUT",
        "/api/v1/logs/levels/cm-test-1",
        TOKEN,
        Some(json!({"level": "debug"})),
    )
    .await;
    assert_eq!(code, 200, "{set}");
    let mut conn = redis.connection();
    let level: Option<String> = redis::cmd("GET")
        .arg(redis.key(&logging::level_key("cm-test-1")))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(level.as_deref(), Some("debug"));
    let (_, levels) = send(http, "GET", "/api/v1/logs/levels", TOKEN, None).await;
    let levels: Value = serde_json::from_str(&levels).unwrap();
    let cm = levels
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["service"] == "cm-test-1")
        .cloned()
        .unwrap();
    assert_eq!(cm["level"], "debug");
    assert!(cm["expires_in_secs"].as_u64().unwrap() > 3000);
    let (code, _) = send(
        http,
        "PUT",
        "/api/v1/logs/levels/cm-test-1",
        TOKEN,
        Some(json!({"level": "loud"})),
    )
    .await;
    assert_eq!(code, 400);
    let (code, _) = send(
        http,
        "PUT",
        "/api/v1/logs/levels/cm-test-1",
        TOKEN,
        Some(json!({"level": null})),
    )
    .await;
    assert_eq!(code, 200);
    let level: Option<String> = redis::cmd("GET")
        .arg(redis.key(&logging::level_key("cm-test-1")))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(level, None);

    admin.stop().await;
    db.drop_db().await;
}
