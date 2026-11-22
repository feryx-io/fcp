//! The dashboard, usage by number, emergency notification calls, vendors'
//! redirection accounts and encryption at rest, route by route
//! (docs/DEEP_TESTING.md D3b).

use super::*;
use fcp_api_core::{CallSupervisor, SupervisionRequest};

/// A record of a call from Alice to `to`, `age` ago.
async fn call_to(app: &App, tenant: TenantId, to: &str, answered_ms: Option<u64>, age: Duration) {
    let mut cdr = cdr_record(tenant, answered_ms, None, SystemTime::now() - age);
    cdr.to_uri = format!("sip:{to}@pbx.example.com");
    app.storage.insert_cdr(cdr).await.expect("insert cdr");
}

/// The dashboard counts what the tenant has, today's calls since midnight
/// UTC and the calls up now, and none of another tenant's.
#[tokio::test]
async fn the_dashboard_counts_the_tenants_own() {
    let app = require_app!();
    for name in ["dee", "dan"] {
        app.create_user_with_role(name, "end_user").await;
    }
    app.create_user_with_role("theirs", "end_user").await;
    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550030001", "tenant_id": app.tenant_a })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);

    // Two answered calls today, one unanswered; one two days ago; one of
    // tenant B's. A call is up.
    // Today's calls must still be today's when the summary is asked for: a
    // run near midnight UTC (one failed at 00:03) waits for the new day.
    let secs_of_day = || {
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            % 86_400
    };
    if secs_of_day() > 86_400 - 120 {
        tokio::time::sleep(Duration::from_secs(86_400 - secs_of_day() + 1)).await;
    }
    let since_midnight = Duration::from_secs(secs_of_day());
    let today = since_midnight.min(Duration::from_secs(60));
    call_to(&app, app.tenant_a, "+15550030001", Some(58_000), today).await;
    call_to(&app, app.tenant_a, "+15550030001", Some(30_000), today).await;
    call_to(&app, app.tenant_a, "+15550030001", None, today).await;
    call_to(
        &app,
        app.tenant_a,
        "+15550030001",
        Some(9_000),
        Duration::from_secs(2 * 86_400),
    )
    .await;
    call_to(&app, app.tenant_b, "+15550030009", Some(5_000), today).await;
    app.call_manager.create_call(Call::new_originated(
        format!("sip:dee@{REALM}"),
        None,
        format!("sip:dan@{REALM}"),
        app.tenant_a,
    ));

    let summary = app
        .admin(Method::GET, "/api/v1/dashboard/summary", None)
        .await;
    assert_eq!(summary.status, StatusCode::OK, "{}", summary.text);
    // The static token's users are the tenant's three plus whoever the
    // harness made; count them as the users route does.
    let users = app.admin(Method::GET, "/api/v1/users", None).await;
    assert_eq!(
        summary.json["total_users"], users.body["total"],
        "{}",
        summary.text
    );
    assert_eq!(summary.json["total_phone_numbers"], 1, "{}", summary.text);
    assert_eq!(summary.json["calls_today"], 3, "{}", summary.text);
    assert_eq!(
        summary.json["avg_call_duration_secs"], 44.0,
        "{}",
        summary.text
    );
    assert_eq!(summary.json["active_calls"], 1, "{}", summary.text);

    let theirs = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/dashboard/summary", None)
        .await;
    assert_eq!(theirs.json["calls_today"], 1, "{}", theirs.text);
    assert_eq!(theirs.json["total_phone_numbers"], 0, "{}", theirs.text);
    assert_eq!(theirs.json["active_calls"], 0, "{}", theirs.text);
    app.finish().await;
}

/// Usage by number covers every number the tenant has calls on — more
/// than a hundred of them, a page at a time — counted in and out.
#[tokio::test]
async fn usage_by_number_covers_every_number() {
    let app = require_app!();
    let numbers: Vec<String> = (0..105).map(|i| format!("+1555004{i:04}")).collect();
    for e164 in &numbers {
        let res = app
            .admin(
                Method::POST,
                "/api/v1/phone-numbers",
                Some(json!({ "e164": e164, "tenant_id": app.tenant_a })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
        call_to(
            &app,
            app.tenant_a,
            e164,
            Some(10_000),
            Duration::from_secs(300),
        )
        .await;
    }
    // The first number twice more, once unanswered; another tenant's call
    // to it is not counted.
    call_to(
        &app,
        app.tenant_a,
        &numbers[0],
        Some(20_000),
        Duration::from_secs(200),
    )
    .await;
    call_to(
        &app,
        app.tenant_a,
        &numbers[0],
        None,
        Duration::from_secs(100),
    )
    .await;
    call_to(
        &app,
        app.tenant_b,
        &numbers[0],
        Some(1_000),
        Duration::from_secs(100),
    )
    .await;

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let path = match &cursor {
            Some(c) => format!("/api/v1/usage/by-number?limit=50&cursor={c}"),
            None => "/api/v1/usage/by-number?limit=50".to_string(),
        };
        let page = app.admin(Method::GET, &path, None).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.text);
        assert_eq!(page.body["total"], 105, "{}", page.text);
        seen.extend(page.json.as_array().unwrap().iter().cloned());
        match page.body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 105);
    let first = seen
        .iter()
        .find(|n| n["number"] == numbers[0].as_str())
        .expect("the first number");
    assert_eq!(first["total_calls"], 3, "{first}");
    assert_eq!(first["answered_calls"], 2, "{first}");
    assert_eq!(first["inbound_calls"], 3, "{first}");
    assert_eq!(first["total_duration_ms"], 30_000, "{first}");

    // A window long past holds none of it.
    let window = app
        .admin(
            Method::GET,
            "/api/v1/usage/by-number?from=2000-01-01&to=2000-01-31",
            None,
        )
        .await;
    assert_eq!(window.status, StatusCode::OK, "{}", window.text);
    assert_eq!(window.body["total"], 0, "{}", window.text);
    app.finish().await;
}

/// A node that places notification calls, answering as the call manager
/// does: what it was asked is kept.
struct NotifyingNode {
    asked: Arc<std::sync::Mutex<Vec<(TenantId, Value)>>>,
}

#[async_trait::async_trait]
impl CallSupervisor for NotifyingNode {
    async fn supervise(
        &self,
        _: TenantId,
        _: &str,
        _: SupervisionRequest,
    ) -> Result<(u16, Value), String> {
        Err("not this".to_string())
    }

    async fn notify_call(&self, tenant: TenantId, request: Value) -> Result<(u16, Value), String> {
        self.asked.lock().unwrap().push((tenant, request.clone()));
        if request["to"] == "9999" {
            return Ok((
                503,
                json!({ "error": "the call could not be placed: 9999 is not one of the tenant's users or extensions" }),
            ));
        }
        Ok((200, json!({ "outcome": "answered", "to": request["to"] })))
    }
}

/// An administrator tries a contact: the notification call is placed by
/// a call manager for the caller's tenant, and what became of it comes
/// back. Without one, or for a contact it cannot ring, they are told.
#[tokio::test]
async fn a_notification_call_is_placed_by_a_call_manager() {
    let app = require_app!();
    let path = "/api/v1/emergency/notify-call";
    let body = json!({ "to": "1050", "text": "An emergency call was placed from 1001." });
    let without = app.admin(Method::POST, path, Some(body.clone())).await;
    assert_eq!(
        without.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        without.text
    );

    let asked: Arc<std::sync::Mutex<Vec<(TenantId, Value)>>> = Arc::default();
    let mut app = app;
    app.state = app
        .state
        .clone()
        .with_call_supervisor(Arc::new(NotifyingNode {
            asked: Arc::clone(&asked),
        }));
    app.router = create_router(app.state.clone(), &test_api_config());

    let placed = app.admin(Method::POST, path, Some(body.clone())).await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.text);
    assert_eq!(placed.json["outcome"], "answered");
    assert_eq!(placed.json["to"], "1050");
    let from_b = app
        .admin_in(app.tenant_b, Method::POST, path, Some(body))
        .await;
    assert_eq!(from_b.status, StatusCode::OK, "{}", from_b.text);
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].0, app.tenant_a);
        assert_eq!(
            asked[0].1["text"],
            "An emergency call was placed from 1001."
        );
        assert_eq!(asked[0].1["repeat"], 2, "the default");
        assert_eq!(asked[1].0, app.tenant_b);
    }

    let unreachable = app
        .admin(
            Method::POST,
            path,
            Some(json!({ "to": "9999", "text": "x" })),
        )
        .await;
    assert_eq!(
        unreachable.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        unreachable.text
    );
    assert!(
        unreachable.text.contains("not one of the tenant's"),
        "{}",
        unreachable.text
    );
    for bad in [
        json!({ "to": " ", "text": "x" }),
        json!({ "to": "1050", "text": "" }),
    ] {
        let res = app.admin(Method::POST, path, Some(bad)).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
    }
    assert_eq!(
        asked.lock().unwrap().len(),
        3,
        "refusals are not put to the node"
    );

    app.create_user_with_role("eu", "end_user").await;
    let eu = session(&app, "eu").await;
    let refused = app
        .send_as(
            &eu,
            Method::POST,
            path,
            Some(json!({ "to": "1050", "text": "x" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    app.finish().await;
}

/// A vendor's redirection account is forgotten in its tenant only.
#[tokio::test]
async fn a_redirection_account_is_forgotten() {
    let app = require_app!();
    let path = "/api/v1/provisioning/redirection/yealink_rps";
    let saved = app
        .admin(
            Method::PUT,
            path,
            Some(json!({ "key_id": "rps-key-id", "secret": "rps-key-secret" })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);

    let from_b = app.admin_in(app.tenant_b, Method::DELETE, path, None).await;
    assert_eq!(from_b.status, StatusCode::NOT_FOUND, "{}", from_b.text);
    let unknown = app
        .admin(
            Method::DELETE,
            "/api/v1/provisioning/redirection/acme_zt",
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text);
    let forgotten = app.admin(Method::DELETE, path, None).await;
    assert_eq!(
        forgotten.status,
        StatusCode::NO_CONTENT,
        "{}",
        forgotten.text
    );
    let listed = app
        .admin(Method::GET, "/api/v1/provisioning/redirection", None)
        .await;
    let rps = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["service"] == "yealink_rps")
        .cloned()
        .unwrap_or(Value::Null);
    assert_ne!(rps["configured"], true, "{}", listed.text);
    let again = app.admin(Method::DELETE, path, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);
    app.finish().await;
}

/// The master key the encryption tests open the database with.
const MASTER_KEY: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

impl App {
    /// The app on the same database with encryption at rest on.
    async fn with_encryption(mut self) -> Self {
        let sealed = Arc::new(
            PostgresStorage::new(
                PostgresConfig::new(db_url(&self.admin_url, &self.db_name)).with_encryption(
                    fcp_crypto::EncryptionSettings::local_from_base64(MASTER_KEY).unwrap(),
                ),
            )
            .await
            .expect("open with a master key"),
        );
        let storage = (*self.dyn_storage)
            .clone()
            .with_encryption_store(sealed as Arc<dyn fcp_core::encryption::EncryptionStore>);
        self.state.storage = Some(Arc::new(storage));
        self.router = create_router(self.state.clone(), &test_api_config());
        self
    }
}

/// The key ring is the platform's: rotated, its secrets sealed again and
/// its keys rewrapped, and a configuration secret sealed, by the platform
/// administrator only — and refused where encryption at rest is off.
#[tokio::test]
async fn encryption_at_rest_is_managed_by_the_platform() {
    let app = require_app!();
    for path in ["rotate", "reencrypt", "rewrap"] {
        let res = app
            .admin(
                Method::POST,
                &format!("/api/v1/system/encryption/{path}"),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{path}: {}", res.text);
    }
    let off = app
        .admin(
            Method::POST,
            "/api/v1/system/encryption/seal",
            Some(json!({ "value": "s3cret" })),
        )
        .await;
    assert_eq!(off.status, StatusCode::CONFLICT, "{}", off.text);

    let app = app.with_encryption().await;
    let before = app
        .admin(Method::GET, "/api/v1/system/encryption", None)
        .await;
    assert_eq!(before.status, StatusCode::OK, "{}", before.text);
    assert_eq!(before.json["enabled"], true, "{}", before.text);
    let first_key = before.json["active_key_id"].clone();

    let rotated = app
        .admin(Method::POST, "/api/v1/system/encryption/rotate", None)
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    assert_ne!(rotated.json["active_key_id"], first_key, "{}", rotated.text);

    let sealed = app
        .admin(
            Method::POST,
            "/api/v1/system/encryption/seal",
            Some(json!({ "value": "validator-token" })),
        )
        .await;
    assert_eq!(sealed.status, StatusCode::OK, "{}", sealed.text);
    let value = sealed.json["sealed"].as_str().unwrap();
    assert!(value.starts_with("enc:v1:"), "{value}");
    assert!(!sealed.text.contains("validator-token"), "{}", sealed.text);
    let empty = app
        .admin(
            Method::POST,
            "/api/v1/system/encryption/seal",
            Some(json!({ "value": "" })),
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);

    let reencrypted = app
        .admin(Method::POST, "/api/v1/system/encryption/reencrypt", None)
        .await;
    assert_eq!(reencrypted.status, StatusCode::OK, "{}", reencrypted.text);
    assert!(
        reencrypted.json["rewritten"].is_u64(),
        "{}",
        reencrypted.text
    );
    let rewrapped = app
        .admin(Method::POST, "/api/v1/system/encryption/rewrap", None)
        .await;
    assert_eq!(rewrapped.status, StatusCode::OK, "{}", rewrapped.text);
    assert_eq!(
        rewrapped.json["rewrapped"], 0,
        "one master key: {}",
        rewrapped.text
    );

    // Nobody else, however admin they are in their tenant.
    app.create_user_with_role("ta", "tenant_admin").await;
    let ta = session(&app, "ta").await;
    for (path, body) in [
        ("rotate", None),
        ("reencrypt", None),
        ("rewrap", None),
        ("seal", Some(json!({ "value": "x" }))),
    ] {
        let res = app
            .send_as(
                &ta,
                Method::POST,
                &format!("/api/v1/system/encryption/{path}"),
                body,
            )
            .await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{path}: {}", res.text);
    }
    let after = app
        .admin(Method::GET, "/api/v1/system/encryption", None)
        .await;
    assert_eq!(after.json["active_key_id"], rotated.json["active_key_id"]);
    app.finish().await;
}

/// Creates racing for one tenant name: one is made, the rest are told 409,
/// never 500 (the name check runs before the insert, so the unique index
/// decides the race).
#[tokio::test]
async fn creates_racing_for_one_tenant_name_conflict() {
    let app = require_app!();
    let body = json!({ "name": "racing-tenant" });
    let sends = (0..8).map(|_| app.admin(Method::POST, "/api/v1/tenants", Some(body.clone())));
    let answers = futures_util::future::join_all(sends).await;
    let made = answers.iter().filter(|r| r.status.is_success()).count();
    for r in &answers {
        assert!(
            r.status.is_success() || r.status == StatusCode::CONFLICT,
            "{} {}",
            r.status,
            r.text
        );
    }
    assert_eq!(made, 1, "exactly one create wins");
}
