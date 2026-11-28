//! The dashboard, usage by number, emergency notification calls, vendors'
//! redirection accounts and encryption at rest, route by route
//! (docs/DEEP_TESTING.md D3b).

use super::*;
use fcp_api_core::{CallSupervisor, SupervisionRequest};
use fcp_core::backup::BackupStore;

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
        self.state.encryption =
            Some(fcp_crypto::EncryptionSettings::local_from_base64(MASTER_KEY).unwrap());
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

/// `docs/SCHEDULED_BACKUPS.md` B1: a platform administrator keeps backup
/// destinations — secrets written and never read back, a password made and
/// shown once when none is given, names unique — and asks for a test and a
/// run, which wait for `fcp-backup`; a tenant's administrator may not.
#[tokio::test]
async fn backup_destinations_are_kept_and_runs_asked_for() {
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    let sftp = json!({
        "name": "Off site",
        "target": { "type": "sftp", "host": "backup.example.com", "user": "fcp",
                    "path": "/srv/fcp",
                    "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\n" },
        "schedule": { "time_of_day": "02:30" },
        "keep": { "daily": 7, "weekly": 4, "monthly": 6 },
    });
    let made = app
        .admin(
            Method::POST,
            "/api/v1/system/backups/destinations",
            Some(sftp.clone()),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = made.json["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("bkd-"), "{id}");
    let generated = made.json["password"].as_str().unwrap().to_string();
    assert_eq!(generated.len(), 64, "a generated password, shown once");
    assert_eq!(made.json["target"]["has_private_key"], true);
    assert_eq!(made.json["target"]["port"], 22);
    assert_eq!(
        made.json["repository"],
        "sftp:fcp@backup.example.com:/srv/fcp"
    );
    assert!(!made.text.contains("abc\n"), "{}", made.text);

    let listed = app
        .admin(Method::GET, "/api/v1/system/backups/destinations", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json[0]["has_password"], true);
    assert!(listed.json[0].get("password").is_none(), "{}", listed.text);
    assert!(!listed.text.contains(&generated) && !listed.text.contains("PRIVATE KEY"));

    // Changed without its secrets: they stay.
    let mut change = sftp.clone();
    change["name"] = json!("Off site, Ohio");
    change["target"]
        .as_object_mut()
        .unwrap()
        .remove("private_key");
    let changed = app
        .admin(
            Method::PUT,
            &format!("/api/v1/system/backups/destinations/{id}"),
            Some(change),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json["target"]["has_private_key"], true);
    assert_eq!(changed.json["has_password"], true);
    let kept = app
        .storage
        .get_backup_destination(&id.trim_start_matches("bkd-").parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        kept.password.as_deref(),
        Some(generated.as_str()),
        "the password kept"
    );

    // Refused: a second of the same name, a bad time, no key, nothing kept.
    for (body, want) in [
        (
            json!({ "name": "off site, ohio", "target": sftp["target"] }),
            StatusCode::CONFLICT,
        ),
        (
            json!({ "name": "B", "target": sftp["target"], "schedule": { "time_of_day": "25:00" } }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "name": "C", "target": { "type": "sftp", "host": "h", "user": "u", "path": "/p" } }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "name": "D", "target": sftp["target"], "keep": { "daily": 0, "weekly": 0, "monthly": 0 } }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "name": "E", "target": sftp["target"], "password": "short" }),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let r = app
            .admin(
                Method::POST,
                "/api/v1/system/backups/destinations",
                Some(body.clone()),
            )
            .await;
        assert_eq!(r.status, want, "{body}: {}", r.text);
    }

    // An S3 bucket, with a password given.
    let s3 = app
        .admin(
            Method::POST,
            "/api/v1/system/backups/destinations",
            Some(json!({
                "name": "Bucket",
                "target": { "type": "s3", "endpoint": "https://s3.example.com", "bucket": "fcp",
                            "prefix": "nightly", "access_key_id": "AKIA",
                            "secret_access_key": "secret-value" },
                "password": "a password of my own choosing",
            })),
        )
        .await;
    assert_eq!(s3.status, StatusCode::CREATED, "{}", s3.text);
    assert!(
        s3.json.get("password").is_none(),
        "only a generated one is shown"
    );
    assert_eq!(
        s3.json["repository"],
        "s3:https://s3.example.com/fcp/nightly"
    );
    assert!(!s3.text.contains("secret-value"));

    // A test and a run: asked for, waiting for the service.
    for (path, kind) in [("test", "test"), ("run", "backup")] {
        let r = app
            .admin(
                Method::POST,
                &format!("/api/v1/system/backups/destinations/{id}/{path}"),
                None,
            )
            .await;
        assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
        assert_eq!(r.json["kind"], kind);
        assert_eq!(r.json["state"], "requested");
        let run = r.json["id"].as_str().unwrap().to_string();
        assert!(run.starts_with("bkr-"));
        let one = app
            .admin(
                Method::GET,
                &format!("/api/v1/system/backups/runs/{run}"),
                None,
            )
            .await;
        assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    }
    let runs = app
        .admin(
            Method::GET,
            &format!("/api/v1/system/backups/runs?destination={id}"),
            None,
        )
        .await;
    assert_eq!(runs.status, StatusCode::OK, "{}", runs.text);
    assert_eq!(runs.json.as_array().map(Vec::len), Some(2), "{}", runs.text);

    // Not a tenant's to see.
    app.create_user_with_role("backupta", "tenant_admin").await;
    let login = app.login("backupta", PASSWORD).await;
    let ta = login.json["token"].as_str().unwrap().to_string();
    let refused = app
        .send_as(
            &ta,
            Method::GET,
            "/api/v1/system/backups/destinations",
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);

    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/system/backups/destinations/{id}"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    let twice = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/system/backups/destinations/{id}"),
            None,
        )
        .await;
    assert_eq!(twice.status, StatusCode::NOT_FOUND, "{}", twice.text);
    app.finish().await;
}

/// `docs/SCHEDULED_BACKUPS.md` B2: the master key downloaded once from the
/// backup page — only where it is local, wrapped in the passphrase as an
/// age file that opens back to the key, the download recorded.
#[tokio::test]
async fn the_master_key_is_downloaded_wrapped_in_a_passphrase() {
    use std::io::Read;
    let Some(app) = App::build(DeploymentMode::default(), None).await else {
        return;
    };
    // Encryption off here: nothing to download.
    let off = app
        .admin(Method::GET, "/api/v1/system/backups/key", None)
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.text);
    assert_eq!(off.json["exportable"], false, "{}", off.text);

    let app = app.with_encryption().await;
    let status = app
        .admin(Method::GET, "/api/v1/system/backups/key", None)
        .await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.text);
    assert_eq!(status.json["provider"], "local");
    assert_eq!(status.json["exportable"], true);
    assert!(status.json["last_exported_at"].is_null());
    let kek_id = status.json["kek_id"].as_str().unwrap().to_string();
    assert!(kek_id.starts_with("local:"), "{kek_id}");

    let short = app
        .admin(
            Method::POST,
            "/api/v1/system/backups/key",
            Some(json!({ "passphrase": "short" })),
        )
        .await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST, "{}", short.text);
    let file = app
        .admin(
            Method::POST,
            "/api/v1/system/backups/key",
            Some(json!({ "passphrase": "correct horse battery staple" })),
        )
        .await;
    assert_eq!(file.status, StatusCode::OK, "{}", file.text);
    assert!(
        file.text.starts_with("-----BEGIN AGE ENCRYPTED FILE-----"),
        "{}",
        file.text
    );
    assert!(!file.text.contains(MASTER_KEY));

    // It opens with the passphrase, as `age -d` would.
    let reader = age::armor::ArmoredReader::new(file.text.as_bytes());
    let decryptor = age::Decryptor::new(reader).expect("an age file");
    let identity = age::scrypt::Identity::new(age::secrecy::SecretString::from(
        "correct horse battery staple".to_string(),
    ));
    let mut plain = String::new();
    decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .expect("the passphrase opens it")
        .read_to_string(&mut plain)
        .unwrap();
    let opened: Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(opened["master_key"], MASTER_KEY);
    assert_eq!(opened["kek_id"], kek_id.as_str());

    let after = app
        .admin(Method::GET, "/api/v1/system/backups/key", None)
        .await;
    assert!(after.json["last_exported_at"].is_string(), "{}", after.text);
    assert_eq!(after.json["last_exported_by"], "static_token");

    app.create_user_with_role("keyta", "tenant_admin").await;
    let login = app.login("keyta", PASSWORD).await;
    let ta = login.json["token"].as_str().unwrap().to_string();
    let refused = app
        .send_as(
            &ta,
            Method::POST,
            "/api/v1/system/backups/key",
            Some(json!({ "passphrase": "correct horse battery staple" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    app.finish().await;
}
