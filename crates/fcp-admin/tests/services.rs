//! The admin's record of every service (`docs/SERVICES.md` phase S2),
//! against a running admin, Postgres and Redis: a service seen from its
//! beacon is listed serving with its checks and load; one whose beacon
//! goes is down since then, back when it returns, stopped when it says so;
//! one that is down can be forgotten and one that is up cannot; the admin
//! lists itself; `/metrics` says who is up; a tenant's administrator sees
//! none of it. Skipped without `FCP_TEST_DATABASE_URL` and
//! `FCP_TEST_REDIS_URL`.

use fcp_core::service_health::{CheckResult, ServiceRecord, ServiceState};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::smoke::{self, free_port, loopback};
use fcp_testkit::{Service, TestDatabase};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-admin");
const TOKEN: &str = "services-static-token-0123456789abcdef";

fn record(id: &str, state: ServiceState) -> ServiceRecord {
    ServiceRecord {
        id: id.to_string(),
        kind: "voicemail".to_string(),
        host: "vm-host".to_string(),
        version: "9.9.9".to_string(),
        started_at: 1_700_000_000,
        region: None,
        member: false,
        http_address: Some("10.0.0.5:8083".to_string()),
        sip_address: None,
        public_url: None,
        state,
        checks: vec![CheckResult {
            name: "tts".to_string(),
            required: false,
            ok: false,
            error: Some("refused".to_string()),
            since: 1_700_000_100,
        }],
        load: BTreeMap::from([("calls".to_string(), 2)]),
        beat_at: 1_700_000_200,
    }
}

/// Write a beacon as a process would, lasting `ttl` seconds.
async fn beat(redis: &fcp_storage_redis::RedisStorage, record: &ServiceRecord, ttl: u64) {
    let mut conn = redis.connection();
    let _: () = redis::cmd("SET")
        .arg(redis.key(&format!("service:{}", record.id)))
        .arg(serde_json::to_string(record).unwrap())
        .arg("EX")
        .arg(ttl)
        .query_async(&mut conn)
        .await
        .unwrap();
}

async fn get(http: u16, path: &str, token: &str) -> (u16, Value) {
    let res = reqwest::Client::new()
        .get(loopback(http, path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let code = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    (
        code,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn service(http: u16, id: &str) -> Option<Value> {
    let (code, body) = get(http, "/api/v1/system/services", TOKEN).await;
    assert_eq!(code, 200, "{body}");
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .cloned()
}

async fn until(http: u16, id: &str, what: &str, check: impl Fn(&Option<Value>) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let s = service(http, id).await;
        if check(&s) {
            return s.unwrap_or(Value::Null);
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}: {s:?}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn state_is(state: &'static str) -> impl Fn(&Option<Value>) -> bool {
    move |s| s.as_ref().is_some_and(|s| s["state"] == state)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_admin_keeps_every_service_it_has_seen() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the services test");
        return;
    };
    let Some(redis_url) = private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the services test");
        return;
    };
    let dir = smoke::temp_dir();
    let http = free_port();
    let config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "services-admin"

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

    // The admin lists itself.
    let me = until(
        http,
        "services-admin",
        "the admin itself",
        state_is("serving"),
    )
    .await;
    assert_eq!(me["kind"], "admin");
    assert_eq!(me["version"], env!("CARGO_PKG_VERSION"));

    // A voicemail server is seen, with its checks and load.
    beat(&redis, &record("vm-1", ServiceState::Degraded), 8).await;
    let vm = until(http, "vm-1", "seen", state_is("degraded")).await;
    assert_eq!(vm["host"], "vm-host");
    assert_eq!(vm["http_address"], "10.0.0.5:8083");
    assert_eq!(vm["load"]["calls"], 2);
    assert_eq!(vm["checks"][0]["name"], "tts");
    assert_eq!(vm["checks"][0]["error"], "refused");
    assert!(
        vm["started_at"].as_str().is_some_and(|t| t.contains('T')),
        "{vm}"
    );
    let first_seen = vm["first_seen"].clone();

    // It may not be forgotten while it is up.
    let res = reqwest::Client::new()
        .delete(loopback(http, "/api/v1/system/services/vm-1"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 409);

    // Its beacon goes: down since then, not gone.
    let vm = until(http, "vm-1", "lost", state_is("down")).await;
    assert!(vm["down_since"].as_str().is_some(), "{vm}");

    // Back.
    beat(&redis, &record("vm-1", ServiceState::Serving), 30).await;
    let vm = until(http, "vm-1", "back", state_is("serving")).await;
    assert_eq!(vm["first_seen"], first_seen, "first seen is kept");
    assert!(vm["down_since"].is_null(), "{vm}");

    // Shut down on purpose.
    beat(&redis, &record("vm-1", ServiceState::Stopping), 8).await;
    until(http, "vm-1", "stopped", state_is("stopped")).await;

    // The summary and the filters.
    let (code, summary) = get(http, "/api/v1/system/services/summary", TOKEN).await;
    assert_eq!(code, 200, "{summary}");
    assert_eq!(summary["redis_reachable"], true, "{summary}");
    assert!(summary["down"].as_u64().unwrap() >= 1, "{summary}");
    assert!(summary["serving"].as_u64().unwrap() >= 1, "{summary}");
    let (_, down) = get(http, "/api/v1/system/services?state=stopped", TOKEN).await;
    assert!(
        down["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["state"] == "stopped"),
        "{down}"
    );
    let (code, one) = get(http, "/api/v1/system/services/vm-1", TOKEN).await;
    assert_eq!(code, 200, "{one}");
    assert_eq!(one["kind"], "voicemail");

    // The metrics say who is up.
    let (code, metrics) = get(http, "/metrics", TOKEN).await;
    assert_eq!(code, 200);
    let metrics = metrics.as_str().unwrap_or_default().to_string();
    assert!(
        metrics.contains(r#"fcp_service_up{id="vm-1",kind="voicemail",site=""} 0"#),
        "{metrics}"
    );
    assert!(
        metrics.contains(r#"fcp_service_up{id="services-admin",kind="admin",site=""} 1"#),
        "{metrics}"
    );

    // Forgotten once down.
    let res = reqwest::Client::new()
        .delete(loopback(http, "/api/v1/system/services/vm-1"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 204);
    let (code, _) = get(http, "/api/v1/system/services/vm-1", TOKEN).await;
    assert_eq!(code, 404);

    // A tenant's administrator sees none of the platform's services.
    let client = reqwest::Client::new();
    let made = client
        .post(loopback(http, "/api/v1/users"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "username": "ta",
            "password": "Str0ng-Passw0rd!",
            "role": "tenant_admin",
            "email": "ta@services.test",
        }))
        .send()
        .await
        .unwrap();
    assert!(made.status().is_success(), "{:?}", made.text().await);
    let signed_in: Value = client
        .post(loopback(http, "/api/v1/auth/login"))
        .json(&json!({"username": "ta", "password": "Str0ng-Passw0rd!"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ta = signed_in["token"]
        .as_str()
        .expect("the tenant admin signs in");
    for path in [
        "/api/v1/system/services",
        "/api/v1/system/services/summary",
        "/api/v1/system/services/services-admin",
    ] {
        let (code, body) = get(http, path, ta).await;
        assert_eq!(code, 403, "{path}: {body}");
    }

    admin.stop().await;
    db.drop_db().await;
}
