//! REST handler tests against a real PostgreSQL instance.
//!
//! These drive the full Axum router produced by [`fcp_api_core::create_router`]
//! (bearer middleware, tenant resolution, handlers, storage) with an
//! in-process request, so they cover the wiring the unit tests in `src/`
//! cannot: auth → `TenantContext` → handler → Postgres → JSON response.
//!
//! Gated on `FCP_TEST_DATABASE_URL` exactly like
//! `fcp-storage-postgres/tests/integration.rs`: each test creates its own
//! database, migrates it through `PostgresStorage::new`, and drops it at the
//! end. Unset the variable and every test returns early with a notice.
//!
//! The SIP side of `ApiState` (transaction/dialog/call managers and the
//! message handler) is built on a capturing transport dispatcher, so no
//! socket is opened and no SIP traffic is generated.

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use bytes::Bytes;
use fcp_api_core::{create_router, ApiState};
use fcp_core::flow_trace::{FlowTraceEntry, FlowTraceEventType, FlowTraceStore};
use fcp_core::permissions::DeploymentMode;
use fcp_core::storage::{CallDetailRecord, CallDisposition, CdrStore, DeviceStore, HangupCause};
use fcp_core::storage::{Registration, RegistrationStore};
use fcp_core::{ApiConfig, CallId, DynStorage, Tenant, TenantId, TenantStore, UserId};
use fcp_events::InMemoryEventBus;
use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use fcp_flow::FlowStore;
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::prelude::{
    DialogManager, SubscriptionManager, TransactionManager, TransportContext, TransportDispatcher,
};
use fcp_sip::{B2buaConfig, CallManager};
use fcp_storage_postgres::api_token::PostgresApiTokenStore;
use fcp_storage_postgres::flow_trace::PostgresFlowTraceStore;
use fcp_storage_postgres::prompt::PostgresPromptStore;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::{Connection, PgConnection};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "test-admin-token";
const PROVISIONING_TOKEN: &str = "test-provisioning-shared-token";
const REALM: &str = "pbx.example.com";

// ─── Harness ─────────────────────────────────────────────────────────────

/// Skip the test (returning early) unless a database is configured.
macro_rules! require_app {
    () => {
        require_app!(DeploymentMode::default())
    };
    ($mode:expr) => {
        match App::with_mode($mode).await {
            Some(app) => app,
            None => {
                eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
                return;
            }
        }
    };
}

/// Transport dispatcher that swallows outbound SIP so the handler can be
/// constructed without a network listener.
#[derive(Default)]
struct NullDispatcher;

#[async_trait::async_trait]
impl TransportDispatcher for NullDispatcher {
    async fn dispatch(&self, _ctx: &TransportContext, _payload: Bytes) -> anyhow::Result<()> {
        Ok(())
    }
}

struct App {
    router: Router,
    storage: Arc<PostgresStorage>,
    traces: Arc<PostgresFlowTraceStore>,
    event_bus: Arc<InMemoryEventBus>,
    /// Upload directories (avatars, greetings, prompts); removed on drop.
    _dir: tempfile::TempDir,
    admin_url: String,
    db_name: String,
    /// Default tenant for the static admin token (`ApiState::default_tenant_id`).
    tenant_a: TenantId,
    /// Second tenant, selected per request with the `x-fcp-tenant-id` header.
    tenant_b: TenantId,
}

impl App {
    async fn with_mode(mode: DeploymentMode) -> Option<Self> {
        let admin_url = std::env::var("FCP_TEST_DATABASE_URL").ok()?;
        let db_name = format!("fcp_test_{}", Uuid::new_v4().simple());

        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connect to admin database");
        sqlx::query(&format!("CREATE DATABASE {db_name}"))
            .execute(&mut admin)
            .await
            .expect("create test database");
        admin.close().await.ok();

        let storage = Arc::new(
            PostgresStorage::new(PostgresConfig::new(db_url(&admin_url, &db_name)))
                .await
                .expect("connect + migrate test database"),
        );

        let tenant_a = seed_tenant(&storage, "tenant-a").await;
        let tenant_b = seed_tenant(&storage, "tenant-b").await;

        // Mirror the production wiring in fcp-admin/src/storage.rs: the API
        // token store is a separate Postgres store attached to DynStorage.
        let api_tokens = Arc::new(PostgresApiTokenStore::new(storage.pool().clone()));
        let traces = Arc::new(PostgresFlowTraceStore::new(storage.pool().clone()));
        let dyn_storage = Arc::new(
            DynStorage::from_all_in_one(Arc::clone(&storage))
                .with_api_token_store(api_tokens)
                .with_flow_trace_store(Arc::clone(&traces) as Arc<dyn FlowTraceStore>)
                .with_prompt_store(Arc::new(PostgresPromptStore::new(storage.pool().clone()))
                    as Arc<dyn fcp_core::prompt::PromptStore>),
        );
        let dir = tempfile::Builder::new()
            .prefix("fcp-api-test-")
            .tempdir()
            .expect("temp dir");
        let event_bus = Arc::new(InMemoryEventBus::new());

        // Runtime managers the queue and presence endpoints need. The queue
        // manager starts with one configured queue in the default tenant.
        let queue_config: fcp_queue::QueueSystemConfig = serde_json::from_value(json!({
            "enabled": true,
            "queues": {
                "support": { "name": "Support", "extension": "7000", "strategy": "round_robin" }
            }
        }))
        .expect("queue config");
        let queue_manager = Arc::new(fcp_queue::QueueManager::new(queue_config, None, tenant_a));
        let presence_manager =
            fcp_presence::PresenceManager::new(fcp_presence::PresenceConfig::default(), None);

        // SIP components with no transport behind them.
        let transaction_manager = Arc::new(TransactionManager::new(
            Arc::new(NullDispatcher) as Arc<dyn TransportDispatcher>
        ));
        let dialog_manager = Arc::new(DialogManager::new());
        let subscription_manager = Arc::new(SubscriptionManager::new());
        let call_manager = Arc::new(CallManager::new());
        let handler = SipMessageHandler::new(
            transaction_manager,
            Arc::clone(&dialog_manager),
            fcp_sip::prelude::SipUri::parse(&format!("sip:test@{REALM}")).unwrap(),
            None,
            "127.0.0.1".to_string(),
            10000,
            None,
            None,
            Arc::clone(&call_manager),
            None,
            Some(Arc::clone(&dyn_storage)),
            None,
            None,
            None,
            B2buaConfig::default(),
            None,
            None,
            None,
            None,
            None,
            None,
            String::new(),
        )
        .expect("build SipMessageHandler")
        .with_default_tenant_id(tenant_a);

        let state = ApiState::new(
            Some(dyn_storage),
            Some(Arc::clone(&storage) as Arc<dyn FlowStore>),
            dialog_manager,
            subscription_manager,
            call_manager,
            Arc::new(handler),
            None,
            Arc::clone(&event_bus) as Arc<dyn fcp_events::EventBus>,
            Some(queue_manager),
            Some(presence_manager),
            REALM.to_string(),
            tenant_a,
        )
        .with_deployment_mode(mode)
        .with_profile_pictures_dir(dir.path().join("avatars"))
        .with_voicemail_greetings_dir(dir.path().join("greetings"))
        .with_prompts_dir(dir.path().join("prompts"));

        let api_config = ApiConfig {
            auth_enabled: true,
            auth_token: Some(ADMIN_TOKEN.to_string()),
            provisioning_auth_token: Some(PROVISIONING_TOKEN.to_string()),
            provisioning_require_https: true,
            ..ApiConfig::default()
        };

        Some(Self {
            router: create_router(state, &api_config),
            storage,
            traces,
            event_bus,
            _dir: dir,
            admin_url,
            db_name,
            tenant_a,
            tenant_b,
        })
    }

    /// Send one request through the router. `body` is JSON-encoded when given.
    async fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> Response {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = match body {
            Some(json) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router never fails");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Response {
            status,
            headers,
            json,
            text,
            bytes: bytes.to_vec(),
        }
    }

    /// Authenticated request as the static admin token in the default tenant.
    async fn admin(&self, method: Method, path: &str, body: Option<Value>) -> Response {
        self.send(method, path, &[(AUTH_HEADER, BEARER)], body)
            .await
    }

    /// Send a request with a raw body and explicit content type.
    async fn send_raw(
        &self,
        token: &str,
        method: Method,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(AUTH_HEADER, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .expect("build request");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router never fails");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        Response {
            status,
            headers,
            json: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            text: String::from_utf8_lossy(&bytes).into_owned(),
            bytes: bytes.to_vec(),
        }
    }

    /// POST a multipart/form-data body.
    async fn send_multipart(&self, token: &str, path: &str, parts: &[Part<'_>]) -> Response {
        const BOUNDARY: &str = "fcp-test-boundary-7d3a1b";
        let mut body = Vec::new();
        for (name, file, bytes) in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            match file {
                Some((filename, content_type)) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        self.send_raw(
            token,
            Method::POST,
            path,
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            body,
        )
        .await
    }

    /// Request with an arbitrary bearer token (JWT session or `fcp_…` token).
    async fn send_as(
        &self,
        token: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Response {
        let header = format!("Bearer {token}");
        self.send(method, path, &[(AUTH_HEADER, header.as_str())], body)
            .await
    }

    /// POST /auth/login for a user in the default tenant.
    async fn login(&self, username: &str, password: &str) -> Response {
        self.send(
            Method::POST,
            "/api/v1/auth/login",
            &[],
            Some(json!({ "username": username, "password": password })),
        )
        .await
    }

    /// Create a user with a role; returns its id.
    async fn create_user_with_role(&self, username: &str, role: &str) -> String {
        let res = self
            .admin(
                Method::POST,
                "/api/v1/users",
                Some(json!({
                    "username": username,
                    "password": PASSWORD,
                    "role": role,
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "create user: {}", res.text);
        res.json["id"].as_str().expect("user id").to_string()
    }

    /// Insert a CDR directly through storage (there is no create endpoint).
    async fn seed_cdr(
        &self,
        tenant: TenantId,
        answered_ms: Option<u64>,
        cost: Option<f64>,
    ) -> CallId {
        let start = SystemTime::now() - Duration::from_secs(300);
        let (disposition, answer_time) = match answered_ms {
            Some(_) => (
                CallDisposition::Answered,
                Some(start + Duration::from_secs(2)),
            ),
            None => (CallDisposition::NoAnswer, None),
        };
        let cdr = CallDetailRecord {
            call_id: CallId::new(),
            tenant_id: tenant,
            sip_call_id: format!("{}@test", Uuid::new_v4()),
            from_uri: "sip:alice@pbx.example.com".to_string(),
            from_display_name: Some("Alice".to_string()),
            to_uri: "sip:+15551234567@pbx.example.com".to_string(),
            to_display_name: None,
            start_time: start,
            answer_time,
            end_time: Some(start + Duration::from_secs(60)),
            setup_duration_ms: Some(2000),
            call_duration_ms: answered_ms,
            total_duration_ms: Some(60_000),
            disposition,
            hangup_cause: HangupCause::NormalClearing,
            hangup_party: None,
            sip_status_code: Some(200),
            sip_reason: None,
            route_id: None,
            gateway_id: None,
            media_anchored: true,
            codec_a_leg: Some("PCMU".to_string()),
            codec_b_leg: Some("PCMU".to_string()),
            rtp_packets_sent: None,
            rtp_packets_received: None,
            rtp_bytes_sent: None,
            rtp_bytes_received: None,
            recording_url: None,
            cost,
            currency: cost.map(|_| "USD".to_string()),
            custom_data: None,
        };
        let id = cdr.call_id;
        self.storage.insert_cdr(cdr).await.expect("insert cdr");
        id
    }

    /// Serve the router on a loopback TCP port (for WebSocket tests, which
    /// cannot go through `oneshot`). Returns the bound port.
    async fn serve(&self) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().unwrap().port();
        let router = self.router.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        port
    }

    /// Insert a registration binding directly through storage.
    async fn seed_registration(&self, tenant: TenantId, aor: &str, contact: &str) {
        let now = SystemTime::now();
        self.storage
            .put_registration(Registration {
                tenant_id: tenant,
                aor: aor.to_string(),
                contact: contact.to_string(),
                expires_at: now + Duration::from_secs(3600),
                call_id: format!("{}@test", Uuid::new_v4()),
                cseq: 1,
                user_agent: Some("fcp-test/1.0".to_string()),
                q: None,
                registered_at: now,
                path: None,
                flow_token: None,
            })
            .await
            .expect("put registration");
    }

    /// Insert one flow-trace entry for a call.
    async fn seed_trace(&self, call_id: &CallId, flow_id: &str, event: FlowTraceEventType) {
        self.traces
            .insert_flow_trace(&FlowTraceEntry {
                id: 0,
                call_id: call_id.to_string(),
                flow_id: flow_id.to_string(),
                flow_name: Some("Main IVR".to_string()),
                node_id: Some("answer".to_string()),
                event_type: event,
                timestamp: SystemTime::now(),
                detail: None,
                flow_version: Some(1),
                elapsed_ms: Some(5),
            })
            .await
            .expect("insert trace");
    }

    /// Authenticated request as the static admin token, scoped to `tenant`.
    async fn admin_in(
        &self,
        tenant: TenantId,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Response {
        let tenant = tenant.to_string();
        self.send(
            method,
            path,
            &[(AUTH_HEADER, BEARER), ("x-fcp-tenant-id", tenant.as_str())],
            body,
        )
        .await
    }

    /// Provisioning request as a phone would send it (HTTPS via proxy header,
    /// shared token, optional per-device token).
    async fn provision(&self, mac: &str, device_token: Option<&str>) -> Response {
        let mut headers = vec![
            ("x-forwarded-proto", "https"),
            ("x-fcp-provisioning-token", PROVISIONING_TOKEN),
        ];
        if let Some(token) = device_token {
            headers.push(("x-fcp-device-provisioning-token", token));
        }
        self.send(
            Method::GET,
            &format!("/api/v1/provisioning/{mac}"),
            &headers,
            None,
        )
        .await
    }

    // ── Fixtures created through the API (so they exercise the handlers) ──

    /// Create a user; returns its id.
    async fn create_user(&self, username: &str, sip_password: Option<&str>) -> String {
        let res = self
            .admin(
                Method::POST,
                "/api/v1/users",
                Some(json!({
                    "username": username,
                    "password": "Str0ng-Passw0rd!",
                    "sip_provisioning_password": sip_password,
                    "display_name": format!("User {username}"),
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "create user: {}", res.text);
        res.json["id"].as_str().expect("user id").to_string()
    }

    /// Create a desk phone with a MAC; returns `(device id, generated token)`.
    async fn create_phone(
        &self,
        name: &str,
        mac: &str,
        owner_id: Option<&str>,
    ) -> (String, String) {
        let res = self
            .admin(
                Method::POST,
                "/api/v1/devices",
                Some(json!({
                    "name": name,
                    "device_type": "desk_phone",
                    "tenant_id": self.tenant_a,
                    "mac_address": mac,
                    "owner_user_id": owner_id,
                    "model": "T54W",
                })),
            )
            .await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "create device: {}",
            res.text
        );
        let id = res.json["id"].as_str().expect("device id").to_string();
        let token = res.json["provisioning_token"]
            .as_str()
            .expect("generated provisioning token")
            .to_string();
        (id, token)
    }

    /// Create a config template in `tenant`; returns its id.
    async fn create_template(&self, tenant: TenantId, body: &str) -> String {
        let res = self
            .admin_in(
                tenant,
                Method::POST,
                "/api/v1/config-templates",
                Some(json!({
                    "name": "Yealink test",
                    "vendor": "yealink",
                    "template_body": body,
                })),
            )
            .await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "create template: {}",
            res.text
        );
        res.json["id"].as_str().expect("template id").to_string()
    }

    async fn patch_device(&self, id: &str, body: Value) -> Response {
        self.admin(Method::PATCH, &format!("/api/v1/devices/{id}"), Some(body))
            .await
    }

    /// Drop the database. Call at the end of a passing test.
    async fn finish(self) {
        drop(self.router);
        self.storage.close().await;
        let mut admin = PgConnection::connect(&self.admin_url)
            .await
            .expect("connect to admin database");
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.db_name))
            .execute(&mut admin)
            .await
            .expect("drop test database");
        admin.close().await.ok();
    }
}

/// One multipart part: `(field name, optional (filename, content type), bytes)`.
type Part<'a> = (&'a str, Option<(&'a str, &'a str)>, &'a [u8]);

const AUTH_HEADER: &str = "authorization";
const PASSWORD: &str = "Str0ng-Passw0rd!";
const BEARER: &str = "Bearer test-admin-token";

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    json: Value,
    text: String,
    bytes: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// Swap the database name in an admin connection string, keeping any query
/// parameters (`?sslmode=…`).
fn db_url(admin_url: &str, database: &str) -> String {
    let (base, query) = match admin_url.split_once('?') {
        Some((b, q)) => (b, format!("?{q}")),
        None => (admin_url, String::new()),
    };
    let (head, _) = base
        .rsplit_once('/')
        .expect("FCP_TEST_DATABASE_URL must contain a database path");
    format!("{head}/{database}{query}")
}

async fn seed_tenant(storage: &PostgresStorage, name: &str) -> TenantId {
    let mut tenant = Tenant::new(name.to_string());
    tenant.sip_domains = vec![format!("{name}.example.com")];
    let id = tenant.id;
    storage.put_tenant(tenant).await.expect("seed tenant");
    id
}

const TEMPLATE: &str = "\
account.1.user_name=${sip_username}
account.1.ha1=${sip_auth_ha1_hash}
account.1.password=${sip_password}
account.1.sip_server=${sip_server}
device.mac=${mac_address}
device.model=${model}
";

// ─── Auth layer ──────────────────────────────────────────────────────────

#[tokio::test]
async fn bearer_middleware_gates_the_api() {
    let app = require_app!();

    let anonymous = app.send(Method::GET, "/api/v1/users", &[], None).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let wrong = app
        .send(
            Method::GET,
            "/api/v1/users",
            &[(AUTH_HEADER, "Bearer not-the-token")],
            None,
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let ok = app.admin(Method::GET, "/api/v1/users", None).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);

    // The platform admin may target another tenant, but only one that exists.
    let missing = app
        .admin_in(TenantId::new(), Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text);

    app.finish().await;
}

// ─── Users ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn users_round_trip_and_duplicate_conflicts() {
    let app = require_app!();

    let id = app.create_user("alice", None).await;
    assert!(id.parse::<UserId>().is_ok(), "user id is a UserId: {id}");

    let duplicate = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(json!({ "username": "alice", "password": "Another-Passw0rd!" })),
        )
        .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT, "{}", duplicate.text);

    let got = app.admin(Method::GET, "/api/v1/users/alice", None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["username"], "alice");
    assert_eq!(got.json["tenant_id"], app.tenant_a.to_string());
    assert_eq!(got.json["enabled"], true);

    // Same username is free in another tenant.
    let other = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/users",
            Some(json!({ "username": "alice", "password": "Another-Passw0rd!" })),
        )
        .await;
    assert_eq!(other.status, StatusCode::CREATED, "{}", other.text);
    assert_eq!(other.json["tenant_id"], app.tenant_b.to_string());

    app.finish().await;
}

// ─── Devices ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn device_create_normalizes_mac_and_issues_a_token() {
    let app = require_app!();
    let owner = app.create_user("bob", None).await;

    let (id, token) = app
        .create_phone("Bob's desk", "AA:BB:CC:DD:EE:01", Some(&owner))
        .await;
    assert!(token.starts_with("fcp_prov_"), "generated token: {token}");

    let got = app
        .admin(Method::GET, &format!("/api/v1/devices/{id}"), None)
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    // The spelling the admin entered is preserved; normalization is applied
    // at lookup time (see the collision below and the provisioning tests).
    assert_eq!(got.json["mac_address"], "AA:BB:CC:DD:EE:01");
    assert_eq!(got.json["has_provisioning_token"], true);
    assert_eq!(
        got.json["provisioning_token"],
        Value::Null,
        "the token is only ever returned at creation"
    );
    assert_eq!(got.json["owner_user_id"], owner);

    // A differently formatted spelling of the same MAC collides.
    let collision = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({
                "name": "Impostor",
                "device_type": "desk_phone",
                "tenant_id": app.tenant_a,
                "mac_address": "aa-bb-cc-dd-ee-01",
            })),
        )
        .await;
    assert_eq!(collision.status, StatusCode::CONFLICT, "{}", collision.text);

    // Listing carries the flag but never the secret.
    let list = app.admin(Method::GET, "/api/v1/devices", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let devices = list.json["devices"]
        .as_array()
        .or_else(|| list.json.as_array())
        .expect("device list");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["has_provisioning_token"], true);
    assert_eq!(devices[0]["provisioning_token"], Value::Null);

    // Regression guard (v0.25.49): the per-user query keeps the token hash.
    let owner_id: UserId = owner.parse().unwrap();
    let by_user = app.storage.list_devices_by_user(&owner_id).await.unwrap();
    assert_eq!(by_user.len(), 1);
    assert!(by_user[0].provisioning_token_hash.is_some());

    app.finish().await;
}

#[tokio::test]
async fn devices_are_scoped_to_the_resolved_tenant() {
    let app = require_app!();
    let (id, _) = app.create_phone("Phone A", "AA:BB:CC:DD:EE:02", None).await;
    let path = format!("/api/v1/devices/{id}");

    let cross = app.admin_in(app.tenant_b, Method::GET, &path, None).await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    let cross_list = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(cross_list.status, StatusCode::OK);
    let devices = cross_list.json["devices"]
        .as_array()
        .or_else(|| cross_list.json.as_array())
        .expect("device list");
    assert!(
        devices.is_empty(),
        "tenant B must not see tenant A's devices"
    );

    let cross_delete = app
        .admin_in(app.tenant_b, Method::DELETE, &path, None)
        .await;
    assert_eq!(cross_delete.status, StatusCode::NOT_FOUND);
    assert_eq!(
        app.admin(Method::GET, &path, None).await.status,
        StatusCode::OK,
        "cross-tenant delete must not remove the device"
    );

    let delete = app.admin(Method::DELETE, &path, None).await;
    assert!(
        delete.status == StatusCode::NO_CONTENT || delete.status == StatusCode::OK,
        "delete: {} {}",
        delete.status,
        delete.text
    );
    assert_eq!(
        app.admin(Method::GET, &path, None).await.status,
        StatusCode::NOT_FOUND
    );

    app.finish().await;
}

#[tokio::test]
async fn device_config_template_must_belong_to_the_tenant() {
    let app = require_app!();
    let (id, _) = app.create_phone("Phone A", "AA:BB:CC:DD:EE:03", None).await;

    let foreign = app.create_template(app.tenant_b, TEMPLATE).await;
    let rejected = app
        .patch_device(&id, json!({ "config_template_id": foreign }))
        .await;
    assert_eq!(
        rejected.status,
        StatusCode::BAD_REQUEST,
        "{}",
        rejected.text
    );

    let local = app.create_template(app.tenant_a, TEMPLATE).await;
    let accepted = app
        .patch_device(&id, json!({ "config_template_id": local }))
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.text);
    assert_eq!(accepted.json["config_template_id"], local);

    // Clearing with an empty string detaches the template.
    let cleared = app
        .patch_device(&id, json!({ "config_template_id": "" }))
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text);
    assert_eq!(cleared.json["config_template_id"], Value::Null);

    app.finish().await;
}

// ─── Device auto-provisioning ────────────────────────────────────────────

#[tokio::test]
async fn provisioning_renders_the_template_for_an_authorized_phone() {
    let app = require_app!();
    let owner = app.create_user("carol", Some("Sip-Secret-123")).await;
    let (id, token) = app
        .create_phone("Carol's phone", "AA:BB:CC:DD:EE:04", Some(&owner))
        .await;
    let template = app.create_template(app.tenant_a, TEMPLATE).await;
    let attached = app
        .patch_device(&id, json!({ "config_template_id": template }))
        .await;
    assert_eq!(attached.status, StatusCode::OK, "{}", attached.text);

    // The phone may spell the MAC however it likes.
    let res = app.provision("aa-bb-cc-dd-ee-04", Some(&token)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(
        res.header("cache-control"),
        Some("no-store, no-cache, max-age=0")
    );
    assert_eq!(res.header("x-content-type-options"), Some("nosniff"));
    assert!(res
        .header("content-type")
        .is_some_and(|ct| ct.starts_with("text/plain")));

    let body = &res.text;
    assert!(body.contains("account.1.user_name=carol"), "{body}");
    assert!(body.contains("account.1.password=Sip-Secret-123"), "{body}");
    assert!(
        body.contains(&format!("account.1.sip_server={REALM}")),
        "{body}"
    );
    assert!(body.contains("device.mac=aabbccddee04"), "{body}");
    assert!(body.contains("device.model=T54W"), "{body}");
    // HA1 is rendered from the (sealed-at-rest) stored hash: 32 hex chars, no placeholder left.
    let ha1 = body
        .lines()
        .find_map(|l| l.strip_prefix("account.1.ha1="))
        .expect("ha1 line");
    assert_eq!(ha1.len(), 32, "ha1 line: {ha1}");
    assert!(
        ha1.chars().all(|c| c.is_ascii_hexdigit()),
        "ha1 line: {ha1}"
    );
    assert!(!body.contains("${"), "unrendered placeholder in {body}");

    app.finish().await;
}

#[tokio::test]
async fn provisioning_failures_are_indistinguishable_404s() {
    let app = require_app!();
    let owner = app.create_user("dave", None).await;
    let (id, token) = app
        .create_phone("Dave's phone", "AA:BB:CC:DD:EE:05", Some(&owner))
        .await;
    let template = app.create_template(app.tenant_a, TEMPLATE).await;
    app.patch_device(&id, json!({ "config_template_id": template }))
        .await;

    // Sanity: the happy path works before we start breaking it.
    let ok = app.provision("aabbccddee05", Some(&token)).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);

    let mut failures: Vec<(&str, Response)> = Vec::new();

    // Shared-token gate (runs before any storage lookup).
    failures.push((
        "no shared token",
        app.send(
            Method::GET,
            "/api/v1/provisioning/aabbccddee05",
            &[
                ("x-forwarded-proto", "https"),
                ("x-fcp-device-provisioning-token", &token),
            ],
            None,
        )
        .await,
    ));
    failures.push((
        "wrong shared token",
        app.send(
            Method::GET,
            "/api/v1/provisioning/aabbccddee05",
            &[
                ("x-forwarded-proto", "https"),
                ("x-fcp-provisioning-token", "wrong-shared-token"),
                ("x-fcp-device-provisioning-token", &token),
            ],
            None,
        )
        .await,
    ));
    // The admin API token must not double as the provisioning token.
    failures.push((
        "admin token as shared token",
        app.send(
            Method::GET,
            "/api/v1/provisioning/aabbccddee05",
            &[
                ("x-forwarded-proto", "https"),
                (AUTH_HEADER, BEARER),
                ("x-fcp-device-provisioning-token", &token),
            ],
            None,
        )
        .await,
    ));
    failures.push((
        "plain http",
        app.send(
            Method::GET,
            "/api/v1/provisioning/aabbccddee05",
            &[
                ("x-fcp-provisioning-token", PROVISIONING_TOKEN),
                ("x-fcp-device-provisioning-token", &token),
            ],
            None,
        )
        .await,
    ));

    // Device-level checks.
    failures.push((
        "unknown MAC",
        app.provision("001122334455", Some(&token)).await,
    ));
    failures.push((
        "wrong device token",
        app.provision("aabbccddee05", Some("fcp_prov_not_the_right_token_at_all"))
            .await,
    ));
    failures.push(("no device token", app.provision("aabbccddee05", None).await));

    // Disabled device with otherwise correct credentials.
    let disabled = app.patch_device(&id, json!({ "enabled": false })).await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.text);
    failures.push((
        "disabled device",
        app.provision("aabbccddee05", Some(&token)).await,
    ));

    // Enabled again but without a template.
    app.patch_device(&id, json!({ "enabled": true, "config_template_id": "" }))
        .await;
    failures.push((
        "no template",
        app.provision("aabbccddee05", Some(&token)).await,
    ));

    for (case, res) in &failures {
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{case}: {}", res.text);
        assert_eq!(
            res.json["details"], "Provisioning config not found",
            "{case}: body must not leak the reason: {}",
            res.text
        );
        assert_eq!(res.header("cache-control"), None, "{case}");
    }
    let bodies: std::collections::HashSet<&str> =
        failures.iter().map(|(_, r)| r.text.as_str()).collect();
    assert_eq!(bodies.len(), 1, "all failure bodies must be identical");

    app.finish().await;
}

// ─── JWT sessions ────────────────────────────────────────────────────────

#[tokio::test]
async fn jwt_login_sessions_and_role_gating() {
    let app = require_app!();
    app.create_user_with_role("ops", "tenant_admin").await;
    app.create_user_with_role("phone-user", "end_user").await;

    let bad = app.login("ops", "not-the-password").await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED, "{}", bad.text);
    let unknown = app.login("nobody", PASSWORD).await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED, "{}", unknown.text);

    let ok = app.login("ops", PASSWORD).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(ok.json["user"]["role"], "tenant_admin");
    assert_eq!(ok.json["user"]["tenant_id"], app.tenant_a.to_string());
    let jwt = ok.json["token"].as_str().expect("jwt").to_string();

    // The session works on the bearer middleware and resolves its tenant.
    let me = app
        .send_as(&jwt, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.text);
    assert_eq!(me.json["username"], "ops");
    let devices = app
        .send_as(&jwt, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(devices.status, StatusCode::OK, "{}", devices.text);

    // A tenant admin is not a platform admin: it sees only its own tenant,
    // cannot create tenants, and the tenant override header is ignored.
    let tenants = app
        .send_as(&jwt, Method::GET, "/api/v1/tenants", None)
        .await;
    assert_eq!(tenants.status, StatusCode::OK, "{}", tenants.text);
    let ids: Vec<&str> = tenants
        .json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![app.tenant_a.to_string().as_str()],
        "{}",
        tenants.text
    );
    let create_tenant = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/tenants",
            Some(json!({ "name": "rogue" })),
        )
        .await;
    assert_eq!(
        create_tenant.status,
        StatusCode::FORBIDDEN,
        "{}",
        create_tenant.text
    );
    let other = app
        .send(
            Method::GET,
            "/api/v1/users",
            &[
                (AUTH_HEADER, &format!("Bearer {jwt}")),
                ("x-fcp-tenant-id", &app.tenant_b.to_string()),
            ],
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::OK, "{}", other.text);
    let names: Vec<&str> = other
        .json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["username"].as_str())
        .collect();
    assert!(
        names.contains(&"ops"),
        "tenant admin sees its own tenant, got {names:?}"
    );

    // Refresh issues a new usable token.
    let refreshed = app
        .send_as(&jwt, Method::POST, "/api/v1/auth/refresh", None)
        .await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.text);
    let jwt2 = refreshed.json["token"]
        .as_str()
        .expect("refreshed jwt")
        .to_string();
    // (Within the same second the claims, and thus the token, may repeat;
    // what matters is that the refreshed token is accepted.)
    let me2 = app
        .send_as(&jwt2, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me2.status, StatusCode::OK, "{}", me2.text);

    // End users authenticate but are kept off admin routes.
    let eu = app.login("phone-user", PASSWORD).await;
    assert_eq!(eu.status, StatusCode::OK, "{}", eu.text);
    let eu_jwt = eu.json["token"].as_str().unwrap().to_string();
    let me = app
        .send_as(&eu_jwt, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.text);
    let denied = app
        .send_as(&eu_jwt, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text);

    // Disabling the account invalidates live sessions immediately.
    let disabled = app
        .admin(
            Method::PUT,
            "/api/v1/users/ops",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.text);
    let dead = app
        .send_as(&jwt2, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(dead.status, StatusCode::UNAUTHORIZED, "{}", dead.text);
    let relogin = app.login("ops", PASSWORD).await;
    assert_eq!(relogin.status, StatusCode::FORBIDDEN, "{}", relogin.text);

    app.finish().await;
}

#[tokio::test]
async fn login_lockout_after_repeated_failures() {
    let app = require_app!();
    app.create_user_with_role("locky", "tenant_admin").await;

    // SecurityPolicy::default() allows 5 failures; each is a generic 401.
    for attempt in 1..=5 {
        let res = app.login("locky", "wrong").await;
        assert_eq!(
            res.status,
            StatusCode::UNAUTHORIZED,
            "attempt {attempt}: {}",
            res.text
        );
    }
    // Now locked: even the right password is refused, and the reason is explicit.
    let locked = app.login("locky", PASSWORD).await;
    assert_eq!(locked.status, StatusCode::FORBIDDEN, "{}", locked.text);

    let unlock = app
        .admin(Method::POST, "/api/v1/users/locky/unlock", None)
        .await;
    assert!(
        unlock.status.is_success(),
        "unlock: {} {}",
        unlock.status,
        unlock.text
    );
    let ok = app.login("locky", PASSWORD).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);

    app.finish().await;
}

// ─── Database API tokens ─────────────────────────────────────────────────

#[tokio::test]
async fn database_api_tokens_authenticate_and_are_scoped() {
    let app = require_app!();
    app.create_phone("Phone", "AA:BB:CC:DD:EE:10", None).await;

    let create = |name: &str, perms: Value, expires: Option<&str>| json!({ "name": name, "permissions": perms, "expires_at": expires });

    // Scoped token: authenticates, but admin routes need "admin"/"*".
    let scoped = app
        .admin(
            Method::POST,
            "/api/v1/api-tokens",
            Some(create("scoped", json!(["device_read"]), None)),
        )
        .await;
    assert_eq!(scoped.status, StatusCode::CREATED, "{}", scoped.text);
    let secret = scoped.json["secret"].as_str().expect("secret").to_string();
    assert!(secret.starts_with("fcp_"), "{secret}");
    let scoped_id = scoped.json["token"]["id"].as_str().unwrap().to_string();
    assert_eq!(scoped.json["token"]["tenant_id"], app.tenant_a.to_string());

    // 403 (not 401) proves the token authenticated but lacks admin rights.
    // (/auth/me is a JWT-session endpoint, so it is not usable as a probe.)
    let denied = app
        .send_as(&secret, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text);

    // Admin token: full access, scoped to its tenant.
    let admin = app
        .admin(
            Method::POST,
            "/api/v1/api-tokens",
            Some(create("automation", json!(["admin"]), None)),
        )
        .await;
    assert_eq!(admin.status, StatusCode::CREATED, "{}", admin.text);
    let admin_secret = admin.json["secret"].as_str().unwrap().to_string();
    let devices = app
        .send_as(&admin_secret, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(devices.status, StatusCode::OK, "{}", devices.text);
    assert_eq!(devices.json.as_array().unwrap().len(), 1);
    // API tokens are never platform admins: they see their own tenant only
    // and cannot create tenants.
    let tenants = app
        .send_as(&admin_secret, Method::GET, "/api/v1/tenants", None)
        .await;
    assert_eq!(tenants.status, StatusCode::OK, "{}", tenants.text);
    assert_eq!(
        tenants.json.as_array().unwrap().len(),
        1,
        "{}",
        tenants.text
    );
    assert_eq!(tenants.json[0]["id"], app.tenant_a.to_string());
    let create_tenant = app
        .send_as(
            &admin_secret,
            Method::POST,
            "/api/v1/tenants",
            Some(json!({ "name": "rogue" })),
        )
        .await;
    assert_eq!(
        create_tenant.status,
        StatusCode::FORBIDDEN,
        "{}",
        create_tenant.text
    );

    // Listing never returns secrets, and the last-used stamp moves.
    let list = app.admin(Method::GET, "/api/v1/api-tokens", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let listed = list.json.as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|t| t.get("secret").is_none()));
    // last_used_at is stamped asynchronously after a successful auth.
    let mut used = listed
        .iter()
        .find(|t| t["name"] == "automation")
        .unwrap()
        .clone();
    for _ in 0..20 {
        if !used["last_used_at"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let again = app.admin(Method::GET, "/api/v1/api-tokens", None).await;
        used = again
            .json
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "automation")
            .unwrap()
            .clone();
    }
    assert!(
        !used["last_used_at"].is_null(),
        "last_used_at not stamped: {used}"
    );

    // Expired tokens are refused; unknown and wrong tokens too.
    let expired = app
        .admin(
            Method::POST,
            "/api/v1/api-tokens",
            Some(create("expired", json!(["admin"]), Some("2020-01-01"))),
        )
        .await;
    assert_eq!(expired.status, StatusCode::CREATED, "{}", expired.text);
    let expired_secret = expired.json["secret"].as_str().unwrap().to_string();
    let refused = app
        .send_as(&expired_secret, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{}", refused.text);
    let bogus = app
        .send_as(
            "fcp_definitely-not-a-token",
            Method::GET,
            "/api/v1/devices",
            None,
        )
        .await;
    assert_eq!(bogus.status, StatusCode::UNAUTHORIZED);

    // Tokens cannot be deleted across tenants; deletion revokes immediately.
    let cross = app
        .admin_in(
            app.tenant_b,
            Method::DELETE,
            &format!("/api/v1/api-tokens/{scoped_id}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);
    let del = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/api-tokens/{scoped_id}"),
            None,
        )
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text);
    let revoked = app
        .send_as(&secret, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(revoked.status, StatusCode::UNAUTHORIZED, "{}", revoked.text);

    app.finish().await;
}

// ─── CDRs, usage, billing ────────────────────────────────────────────────

#[tokio::test]
async fn cdrs_usage_and_billing_summary() {
    let app = require_app!();
    let a1 = app.seed_cdr(app.tenant_a, Some(58_000), Some(0.25)).await;
    let _a2 = app.seed_cdr(app.tenant_a, Some(30_000), Some(0.10)).await;
    let _a3 = app.seed_cdr(app.tenant_a, None, None).await;
    let b1 = app.seed_cdr(app.tenant_b, Some(10_000), Some(1.00)).await;

    // The platform token is a cross-tenant reporting identity: with no
    // tenant selected it sees every tenant's records...
    let all = app.admin(Method::GET, "/api/v1/cdrs", None).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    assert_eq!(all.json["total"], 4, "{}", all.text);
    let by_param = app
        .admin(
            Method::GET,
            &format!("/api/v1/cdrs?tenant_id={}", app.tenant_b),
            None,
        )
        .await;
    assert_eq!(by_param.json["total"], 1, "{}", by_param.text);

    // ...and the override header scopes it like every other endpoint.
    let list = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/cdrs", None)
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.json["total"], 3, "{}", list.text);
    assert_eq!(list.json["cdrs"].as_array().unwrap().len(), 3);

    let answered = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/cdrs?disposition=answered&limit=1",
            None,
        )
        .await;
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.text);
    assert_eq!(answered.json["total"], 2, "{}", answered.text);
    assert_eq!(
        answered.json["cdrs"].as_array().unwrap().len(),
        1,
        "limit applies"
    );
    assert_eq!(answered.json["cdrs"][0]["disposition"], "answered");

    let one = app
        .admin(Method::GET, &format!("/api/v1/cdrs/{a1}"), None)
        .await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    assert_eq!(one.json["cost"], 0.25);
    assert_eq!(one.json["currency"], "USD");
    assert_eq!(one.json["call_duration_ms"], 58_000);

    // Single records follow the same rule: unscoped platform access finds
    // B's record, a tenant-A-scoped request does not.
    let any = app
        .admin(Method::GET, &format!("/api/v1/cdrs/{b1}"), None)
        .await;
    assert_eq!(any.status, StatusCode::OK, "{}", any.text);
    let cross = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/cdrs/{b1}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);
    let b_list = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/cdrs", None)
        .await;
    assert_eq!(b_list.status, StatusCode::OK, "{}", b_list.text);
    assert_eq!(b_list.json["total"], 1, "{}", b_list.text);

    // Usage aggregates the same rows.
    let usage = app
        .admin(Method::GET, "/api/v1/usage?period=current_month", None)
        .await;
    assert_eq!(usage.status, StatusCode::OK, "{}", usage.text);
    assert_eq!(usage.json["total_calls"], 3, "{}", usage.text);
    assert_eq!(usage.json["answered_calls"], 2);
    assert_eq!(usage.json["total_duration_ms"], 88_000);
    assert!(
        (usage.json["total_cost"].as_f64().unwrap() - 0.35).abs() < 1e-9,
        "{}",
        usage.text
    );

    let summary = app
        .admin(Method::GET, "/api/v1/billing/summary", None)
        .await;
    assert_eq!(summary.status, StatusCode::OK, "{}", summary.text);
    assert_eq!(summary.json["usage"]["total_calls"], 3);
    assert_eq!(summary.json["resource_counts"]["users"], 0);

    app.finish().await;
}

#[tokio::test]
async fn rate_plans_and_balance_ledger() {
    let app = require_app!();

    let plan = app
        .admin(
            Method::POST,
            "/api/v1/rate-plans",
            Some(json!({
                "name": "Standard",
                "currency": "USD",
                "default_rate_per_minute": 0.02,
                "connect_fee": 0.0,
                "billing_increment_secs": 6,
                "is_default": true,
            })),
        )
        .await;
    assert_eq!(plan.status, StatusCode::CREATED, "{}", plan.text);
    let plan_id = plan.json["id"].as_str().unwrap().to_string();
    assert_eq!(plan.json["tenant_id"], app.tenant_a.to_string());

    let rate = app
        .admin(
            Method::POST,
            &format!("/api/v1/rate-plans/{plan_id}/rates"),
            Some(json!({ "prefix": "1555", "rate_per_minute": 0.05, "description": "test range" })),
        )
        .await;
    assert_eq!(rate.status, StatusCode::CREATED, "{}", rate.text);
    let rate_id = rate.json["id"].as_str().unwrap().to_string();
    let rates = app
        .admin(
            Method::GET,
            &format!("/api/v1/rate-plans/{plan_id}/rates"),
            None,
        )
        .await;
    assert_eq!(rates.json.as_array().unwrap().len(), 1, "{}", rates.text);

    let renamed = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/rate-plans/{plan_id}"),
            Some(json!({ "name": "Standard v2" })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text);
    assert_eq!(renamed.json["name"], "Standard v2");

    // Other tenants cannot see or touch the plan.
    let cross = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/rate-plans/{plan_id}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    let del_rate = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/rate-plans/{plan_id}/rates/{rate_id}"),
            None,
        )
        .await;
    assert_eq!(del_rate.status, StatusCode::NO_CONTENT, "{}", del_rate.text);
    let del_plan = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/rate-plans/{plan_id}"),
            None,
        )
        .await;
    assert_eq!(del_plan.status, StatusCode::NO_CONTENT, "{}", del_plan.text);
    let gone = app
        .admin(Method::GET, &format!("/api/v1/rate-plans/{plan_id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    // No balance row exists until it is configured; the first PUT creates it.
    let balance = app
        .admin(Method::GET, "/api/v1/billing/balance", None)
        .await;
    assert_eq!(balance.status, StatusCode::NOT_FOUND, "{}", balance.text);

    let settings = app
        .admin(
            Method::PUT,
            "/api/v1/billing/balance",
            Some(json!({ "credit_limit": 10.0, "enforce": true })),
        )
        .await;
    assert_eq!(
        settings.status,
        StatusCode::CREATED,
        "first PUT creates the row: {}",
        settings.text
    );
    assert_eq!(settings.json["credit_limit"], 10.0);
    assert_eq!(settings.json["enforce"], true);
    assert_eq!(settings.json["balance"], 0.0);
    let again = app
        .admin(
            Method::PUT,
            "/api/v1/billing/balance",
            Some(json!({ "low_balance_threshold": 5.0 })),
        )
        .await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "second PUT updates: {}",
        again.text
    );
    assert_eq!(again.json["low_balance_threshold"], 5.0);
    assert_eq!(again.json["credit_limit"], 10.0, "unchanged fields persist");

    let top_up = app
        .admin(
            Method::POST,
            "/api/v1/billing/balance/transactions",
            Some(json!({ "kind": "top_up", "amount": 25.0, "description": "prepay" })),
        )
        .await;
    assert_eq!(top_up.status, StatusCode::CREATED, "{}", top_up.text);
    assert_eq!(top_up.json["balance_after"], 25.0);

    let negative_top_up = app
        .admin(
            Method::POST,
            "/api/v1/billing/balance/transactions",
            Some(json!({ "kind": "top_up", "amount": -5.0 })),
        )
        .await;
    assert_eq!(
        negative_top_up.status,
        StatusCode::BAD_REQUEST,
        "{}",
        negative_top_up.text
    );

    let adjustment = app
        .admin(
            Method::POST,
            "/api/v1/billing/balance/transactions",
            Some(
                json!({ "kind": "adjustment", "amount": -5.0, "description": "goodwill reversal" }),
            ),
        )
        .await;
    assert_eq!(
        adjustment.status,
        StatusCode::CREATED,
        "{}",
        adjustment.text
    );
    assert_eq!(adjustment.json["balance_after"], 20.0);

    let after = app
        .admin(Method::GET, "/api/v1/billing/balance", None)
        .await;
    assert_eq!(after.json["balance"], 20.0, "{}", after.text);
    assert_eq!(
        after.json["available"], 30.0,
        "balance plus credit limit: {}",
        after.text
    );
    let ledger = app
        .admin(Method::GET, "/api/v1/billing/balance/transactions", None)
        .await;
    assert_eq!(ledger.status, StatusCode::OK, "{}", ledger.text);
    assert_eq!(ledger.json.as_array().unwrap().len(), 2, "{}", ledger.text);

    // Tenant B's ledger is untouched (still unconfigured).
    let b = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/billing/balance", None)
        .await;
    assert_eq!(b.status, StatusCode::NOT_FOUND, "{}", b.text);

    app.finish().await;
}

// ─── Trunks ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn trunks_crud_hides_secrets_and_respects_deployment_mode() {
    // Carrier mode so a tenant admin session may manage trunks.
    let app = require_app!(DeploymentMode::Carrier);

    let created = app
        .admin(
            Method::POST,
            "/api/v1/trunks",
            Some(json!({
                "name": "upstream",
                "hosts": ["sip.carrier.example.net"],
                "transport_protocol": "udp",
                "auth_username": "acct1",
                "auth_password": "trunk-secret-pw",
                "auth_realm": "carrier.example.net",
                "max_concurrent_calls": 30,
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let id = created.json["id"].as_str().unwrap().to_string();
    assert_eq!(created.json["has_credentials"], true);
    assert_eq!(created.json["auth_username"], "acct1");
    assert!(
        !created.text.contains("trunk-secret-pw"),
        "password must never be echoed: {}",
        created.text
    );

    let got = app
        .admin(Method::GET, &format!("/api/v1/trunks/{id}"), None)
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert!(!got.text.contains("trunk-secret-pw"));

    let updated = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/trunks/{id}"),
            Some(json!({ "max_concurrent_calls": 45, "enabled": false })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json["max_concurrent_calls"], 45);
    assert_eq!(updated.json["enabled"], false);
    assert_eq!(
        updated.json["has_credentials"], true,
        "update must keep the sealed password"
    );

    // A tenant admin session can manage trunks in Carrier mode.
    app.create_user_with_role("carrier-ops", "tenant_admin")
        .await;
    let jwt = app.login("carrier-ops", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = app.send_as(&jwt, Method::GET, "/api/v1/trunks", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json.as_array().unwrap().len(), 1);

    let cross = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/trunks/{id}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    let del = app
        .admin(Method::DELETE, &format!("/api/v1/trunks/{id}"), None)
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text);
    assert_eq!(
        app.admin(Method::GET, &format!("/api/v1/trunks/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    app.finish().await;
}

#[tokio::test]
async fn trunks_are_closed_to_tenant_admins_in_enterprise_mode() {
    let app = require_app!(DeploymentMode::Enterprise);
    app.create_user_with_role("ent-ops", "tenant_admin").await;
    let jwt = app.login("ent-ops", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();

    let listed = app.send_as(&jwt, Method::GET, "/api/v1/trunks", None).await;
    assert_eq!(listed.status, StatusCode::FORBIDDEN, "{}", listed.text);
    let created = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/trunks",
            Some(json!({ "name": "nope", "hosts": ["sip.example.net"] })),
        )
        .await;
    assert_eq!(created.status, StatusCode::FORBIDDEN, "{}", created.text);

    // The static platform token is unrestricted regardless of mode.
    let listed = app.admin(Method::GET, "/api/v1/trunks", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);

    app.finish().await;
}

// ─── Queues ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn queue_configs_crud_and_runtime_absent() {
    let app = require_app!();
    let agent = app.create_user("agent1", None).await;

    let created = app
        .admin(
            Method::POST,
            "/api/v1/queue-configs",
            Some(json!({
                "name": "billing",
                "extension": "7100",
                "strategy": "ring_all",
                "max_wait_time_secs": 120,
                "agents": [agent],
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let id = created.json["id"].as_str().unwrap().to_string();
    assert_eq!(created.json["name"], "billing");
    assert_eq!(created.json["extension"], "7100");

    let list = app.admin(Method::GET, "/api/v1/queue-configs", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.json.as_array().unwrap().len(), 1);

    let cross = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/queue-configs/{id}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    // Stored configs are not runtime queues until synced: the listing comes
    // from the live QueueManager, and runtime-only endpoints answer 404 (not
    // 500) for a config it has not loaded.
    let queues = app.admin(Method::GET, "/api/v1/queues", None).await;
    assert_eq!(queues.status, StatusCode::OK, "{}", queues.text);
    let names: Vec<&str> = queues
        .json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|q| q["name"].as_str())
        .collect();
    assert!(names.contains(&"support"), "{names:?}");
    let stats = app
        .admin(Method::GET, "/api/v1/queues/billing/stats", None)
        .await;
    assert_eq!(stats.status, StatusCode::NOT_FOUND, "{}", stats.text);

    let del = app
        .admin(Method::DELETE, &format!("/api/v1/queue-configs/{id}"), None)
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text);

    app.finish().await;
}

// ─── Voicemail ───────────────────────────────────────────────────────────

#[tokio::test]
async fn voicemail_mailboxes_crud() {
    let app = require_app!();
    let owner = app.create_user("vm-owner", None).await;

    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550001111", "tenant_id": app.tenant_a })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
    let number_id = number.json["id"].as_str().unwrap().to_string();

    let binding = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({
                "phone_number_id": number_id,
                "owner_user_id": owner,
                "binding_type": "user",
                "tenant_id": app.tenant_a,
            })),
        )
        .await;
    assert_eq!(binding.status, StatusCode::CREATED, "{}", binding.text);
    let binding_id = binding.json["id"].as_str().unwrap().to_string();

    let mailbox = app
        .admin(
            Method::POST,
            "/api/v1/voicemail/mailboxes",
            Some(json!({
                "tenant_id": app.tenant_a,
                "user_id": owner,
                "line_binding_id": binding_id,
                "pin": "4321",
                "max_messages": 25,
            })),
        )
        .await;
    assert_eq!(mailbox.status, StatusCode::CREATED, "{}", mailbox.text);
    let id = mailbox.json["id"].as_str().unwrap().to_string();
    assert_eq!(mailbox.json["user_id"], owner);
    assert_eq!(mailbox.json["max_messages"], 25);
    assert!(
        mailbox.json.get("pin").is_none() && !mailbox.text.contains("4321"),
        "pin must not be echoed: {}",
        mailbox.text
    );

    let got = app
        .admin(
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            None,
        )
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["enabled"], true);

    let disabled = app
        .admin(
            Method::POST,
            &format!("/api/v1/voicemail/mailboxes/{id}/disable"),
            None,
        )
        .await;
    assert!(
        disabled.status.is_success(),
        "disable: {} {}",
        disabled.status,
        disabled.text
    );
    let got = app
        .admin(
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            None,
        )
        .await;
    assert_eq!(got.json["enabled"], false, "{}", got.text);

    let updated = app
        .admin(
            Method::PUT,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            Some(json!({ "max_messages": 50, "email_notification": true, "email_address": "vm@example.com" })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json["max_messages"], 50);
    assert_eq!(updated.json["email_address"], "vm@example.com");

    let stats = app
        .admin(
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{id}/stats"),
            None,
        )
        .await;
    assert_eq!(stats.status, StatusCode::OK, "{}", stats.text);

    let cross = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    let del = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            None,
        )
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text);
    assert_eq!(
        app.admin(
            Method::GET,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    app.finish().await;
}

// ─── Flows and flow traces ───────────────────────────────────────────────

fn hangup_flow(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "initial_node": "answer",
        "nodes": {
            "answer": { "id": "answer", "name": "Answer", "type": "answer", "next": "hangup" },
            "hangup": { "id": "hangup", "name": "Hangup", "type": "hangup", "reason": "normal" }
        },
        "variables": {},
        "enabled": true
    })
}

#[tokio::test]
async fn flows_crud_versions_and_tenant_scoping() {
    let app = require_app!();

    let created = app
        .admin(
            Method::POST,
            "/api/v1/flows",
            Some(hangup_flow("main-ivr", "Main IVR")),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    assert_eq!(created.json["id"], "main-ivr");
    assert_eq!(created.json["tenant_id"], app.tenant_a.to_string());
    assert_eq!(created.json["initial_node"], "answer");

    let got = app.admin(Method::GET, "/api/v1/flows/main-ivr", None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["nodes"]["hangup"]["type"], "hangup");

    let list = app.admin(Method::GET, "/api/v1/flows", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.json["total"], 1, "{}", list.text);
    assert_eq!(list.json["flows"][0]["id"], "main-ivr");

    let updated = app
        .admin(
            Method::PUT,
            "/api/v1/flows/main-ivr",
            Some(json!({ "name": "Main IVR v2", "enabled": false })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json["name"], "Main IVR v2");
    assert_eq!(updated.json["enabled"], false);

    let versions = app
        .admin(Method::GET, "/api/v1/flows/main-ivr/versions", None)
        .await;
    assert_eq!(versions.status, StatusCode::OK, "{}", versions.text);
    let version_count = versions
        .json
        .as_array()
        .map(|v| v.len())
        .or_else(|| versions.json["versions"].as_array().map(|v| v.len()))
        .unwrap_or(0);
    assert!(version_count >= 1, "{}", versions.text);

    // Tenant B cannot see, edit, or delete it.
    let cross = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/flows/main-ivr", None)
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);
    let cross_del = app
        .admin_in(app.tenant_b, Method::DELETE, "/api/v1/flows/main-ivr", None)
        .await;
    assert_eq!(
        cross_del.status,
        StatusCode::NOT_FOUND,
        "{}",
        cross_del.text
    );
    let b_list = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/flows", None)
        .await;
    assert_eq!(b_list.json["total"], 0, "{}", b_list.text);

    let del = app
        .admin(Method::DELETE, "/api/v1/flows/main-ivr", None)
        .await;
    assert!(
        del.status.is_success(),
        "delete: {} {}",
        del.status,
        del.text
    );
    assert_eq!(
        app.admin(Method::GET, "/api/v1/flows/main-ivr", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    app.finish().await;
}

#[tokio::test]
async fn flow_traces_follow_call_visibility() {
    let app = require_app!();
    let a_call = app.seed_cdr(app.tenant_a, Some(5_000), None).await;
    let b_call = app.seed_cdr(app.tenant_b, Some(5_000), None).await;
    app.seed_trace(&a_call, "main-ivr", FlowTraceEventType::Started)
        .await;
    app.seed_trace(&a_call, "main-ivr", FlowTraceEventType::Completed)
        .await;
    app.seed_trace(&b_call, "other-ivr", FlowTraceEventType::Started)
        .await;

    // Scoped to tenant A: only A's call shows, and B's call is invisible.
    let list = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/flow-traces", None)
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.json["total"], 2, "{}", list.text);
    assert!(list.json["traces"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["call_id"] == a_call.to_string()));

    let filtered = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/flow-traces?event_type=completed",
            None,
        )
        .await;
    assert_eq!(filtered.json["total"], 1, "{}", filtered.text);
    let bad = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/flow-traces?event_type=exploded",
            None,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    let by_call = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/flow-traces/{a_call}"),
            None,
        )
        .await;
    assert_eq!(by_call.status, StatusCode::OK, "{}", by_call.text);
    assert_eq!(by_call.json.as_array().unwrap().len(), 2);
    let cross = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/flow-traces/{b_call}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    // Unscoped platform access sees everything.
    let all = app.admin(Method::GET, "/api/v1/flow-traces", None).await;
    assert_eq!(all.json["total"], 3, "{}", all.text);

    app.finish().await;
}

// ─── Registrations ───────────────────────────────────────────────────────

#[tokio::test]
async fn registrations_list_get_and_delete() {
    let app = require_app!();
    let aor = format!("alice@{REALM}");
    app.seed_registration(app.tenant_a, &aor, "sip:alice@10.0.0.5:5060")
        .await;
    app.seed_registration(app.tenant_a, &aor, "sip:alice@10.0.0.6:5060")
        .await;
    app.seed_registration(
        app.tenant_b,
        &format!("bob@{REALM}"),
        "sip:bob@10.0.1.5:5060",
    )
    .await;

    // AORs are visible to a tenant through its users (username@realm).
    app.create_user("alice", None).await;
    app.admin_in(
        app.tenant_b,
        Method::POST,
        "/api/v1/users",
        Some(json!({ "username": "bob", "password": PASSWORD })),
    )
    .await;

    // Unscoped platform view sees both tenants; the tenant view only its own.
    let all = app.admin(Method::GET, "/api/v1/registrations", None).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    assert_eq!(all.json.as_array().unwrap().len(), 3, "{}", all.text);
    let scoped = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/registrations", None)
        .await;
    assert_eq!(scoped.json.as_array().unwrap().len(), 2, "{}", scoped.text);

    let mine = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/registrations/{aor}"),
            None,
        )
        .await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.text);
    let bindings = mine.json.as_array().unwrap();
    assert_eq!(bindings.len(), 2, "{}", mine.text);
    assert!(bindings
        .iter()
        .all(|b| b["expires_in_seconds"].as_u64().unwrap() > 0));
    assert!(bindings.iter().all(|b| b["user_agent"] == "fcp-test/1.0"));

    // Tenant A cannot see tenant B's binding.
    let cross = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/registrations/bob@{REALM}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    // Delete one contact, then the rest.
    let del_one = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/registrations/{aor}/sip:alice@10.0.0.5:5060"),
            None,
        )
        .await;
    assert_eq!(del_one.status, StatusCode::NO_CONTENT, "{}", del_one.text);
    let left = app
        .admin(Method::GET, &format!("/api/v1/registrations/{aor}"), None)
        .await;
    assert_eq!(left.json.as_array().unwrap().len(), 1, "{}", left.text);
    assert_eq!(left.json[0]["contact"], "sip:alice@10.0.0.6:5060");

    let del_all = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/registrations/{aor}"),
            None,
        )
        .await;
    assert_eq!(del_all.status, StatusCode::NO_CONTENT, "{}", del_all.text);
    // A known AOR with no bindings is an empty list, not 404.
    let gone = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/registrations/{aor}"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::OK, "{}", gone.text);
    assert!(gone.json.as_array().unwrap().is_empty(), "{}", gone.text);
    // Tenant B's binding survived the tenant-A deletes.
    let bobs = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/registrations/bob@{REALM}"),
            None,
        )
        .await;
    assert_eq!(bobs.json.as_array().unwrap().len(), 1, "{}", bobs.text);

    app.finish().await;
}

// ─── Presence runtime ────────────────────────────────────────────────────

#[tokio::test]
async fn presence_state_policy_and_pres_rules() {
    let app = require_app!();
    let aor = format!("alice@{REALM}");
    let path = format!("/api/v1/presence/{aor}");

    let invalid = app
        .admin(Method::PUT, &path, Some(json!({ "state": "teleporting" })))
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{}", invalid.text);

    let set = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({ "state": "busy", "note": "In a meeting" })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(set.json["aor"], aor);
    assert!(
        set.json["state"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case("busy"),
        "{}",
        set.text
    );
    assert_eq!(set.json["note"], "In a meeting");

    let got = app.admin(Method::GET, &path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["watcher_count"], 0);
    let list = app.admin(Method::GET, "/api/v1/presence", None).await;
    assert_eq!(list.json.as_array().unwrap().len(), 1, "{}", list.text);
    let stats = app.admin(Method::GET, "/api/v1/presence/stats", None).await;
    assert_eq!(stats.json["total_presentities"], 1, "{}", stats.text);
    let watchers = app
        .admin(Method::GET, &format!("{path}/watchers"), None)
        .await;
    assert_eq!(watchers.status, StatusCode::OK, "{}", watchers.text);
    assert!(watchers.json.as_array().unwrap().is_empty());

    // Authorization policy round-trips and renders as pres-rules XML.
    let policy = json!({
        "default_allow": false,
        "allow_list": [format!("bob@{REALM}")],
        "block_list": [format!("mallory@{REALM}")]
    });
    let set_policy = app
        .admin(Method::PUT, &format!("{path}/policy"), Some(policy.clone()))
        .await;
    assert_eq!(set_policy.status, StatusCode::OK, "{}", set_policy.text);
    let get_policy = app
        .admin(Method::GET, &format!("{path}/policy"), None)
        .await;
    assert_eq!(
        get_policy.json["default_allow"], false,
        "{}",
        get_policy.text
    );
    assert_eq!(get_policy.json["allow_list"][0], format!("bob@{REALM}"));

    let rules = app
        .admin(Method::GET, &format!("{path}/pres-rules"), None)
        .await;
    assert_eq!(rules.status, StatusCode::OK, "{}", rules.text);
    assert_eq!(
        rules.header("content-type"),
        Some("application/auth-policy+xml")
    );
    assert!(rules.text.contains("bob@"), "{}", rules.text);

    let del_policy = app
        .admin(Method::DELETE, &format!("{path}/policy"), None)
        .await;
    assert_eq!(
        del_policy.status,
        StatusCode::NO_CONTENT,
        "{}",
        del_policy.text
    );

    let del = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text);
    assert_eq!(
        app.admin(Method::GET, &path, None).await.status,
        StatusCode::NOT_FOUND
    );
    let missing = app
        .admin(
            Method::GET,
            &format!("/api/v1/presence/nobody@{REALM}"),
            None,
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    app.finish().await;
}

// ─── Queue runtime ───────────────────────────────────────────────────────

#[tokio::test]
async fn queue_runtime_agents_and_stats() {
    let app = require_app!();

    let queues = app.admin(Method::GET, "/api/v1/queues", None).await;
    assert_eq!(queues.status, StatusCode::OK, "{}", queues.text);
    assert_eq!(queues.json[0]["name"], "support", "{}", queues.text);
    assert_eq!(queues.json[0]["extension"], "7000");

    let stats = app
        .admin(Method::GET, "/api/v1/queues/support/stats", None)
        .await;
    assert_eq!(stats.status, StatusCode::OK, "{}", stats.text);
    assert_eq!(stats.json["queue_name"], "support");
    assert_eq!(stats.json["agents_total"], 0);
    assert_eq!(stats.json["calls_waiting"], 0);

    let unknown = app
        .admin(Method::GET, "/api/v1/queues/sales/stats", None)
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text);
    // The queue belongs to tenant A only.
    let cross = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/queues/support", None)
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    let login = app
        .admin(
            Method::POST,
            "/api/v1/queues/support/agents",
            Some(json!({ "agent_id": "agent-1", "extension": "1001", "skills": ["billing"] })),
        )
        .await;
    assert_eq!(login.status, StatusCode::CREATED, "{}", login.text);
    assert_eq!(login.json["id"], "agent-1");
    assert_eq!(login.json["extension"], "1001");
    assert!(login.json["queues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q == "support"));

    let agents = app
        .admin(Method::GET, "/api/v1/queues/support/agents", None)
        .await;
    assert_eq!(agents.json.as_array().unwrap().len(), 1, "{}", agents.text);
    let stats = app
        .admin(Method::GET, "/api/v1/queues/support/stats", None)
        .await;
    assert_eq!(stats.json["agents_total"], 1, "{}", stats.text);

    let paused = app
        .admin(
            Method::POST,
            "/api/v1/queues/support/agents/agent-1/pause",
            None,
        )
        .await;
    assert_eq!(paused.status, StatusCode::OK, "{}", paused.text);
    assert!(
        paused.json["state"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case("paused"),
        "{}",
        paused.text
    );
    let resumed = app
        .admin(
            Method::POST,
            "/api/v1/queues/support/agents/agent-1/resume",
            None,
        )
        .await;
    assert_eq!(resumed.status, StatusCode::OK, "{}", resumed.text);
    assert!(
        resumed.json["state"]
            .as_str()
            .unwrap()
            .eq_ignore_ascii_case("available"),
        "{}",
        resumed.text
    );

    let logout = app
        .admin(
            Method::DELETE,
            "/api/v1/queues/support/agents/agent-1",
            None,
        )
        .await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT, "{}", logout.text);
    // Logout keeps the agent record (for reporting) but marks it logged out
    // and takes it out of the available pool.
    let agents = app
        .admin(Method::GET, "/api/v1/queues/support/agents", None)
        .await;
    let remaining = agents.json.as_array().unwrap();
    assert!(
        remaining.is_empty()
            || remaining.iter().all(|a| a["state"]
                .as_str()
                .unwrap()
                .eq_ignore_ascii_case("logged_out")),
        "{}",
        agents.text
    );
    let stats = app
        .admin(Method::GET, "/api/v1/queues/support/stats", None)
        .await;
    assert_eq!(stats.json["agents_available"], 0, "{}", stats.text);

    app.finish().await;
}

// ─── Self-service (/me) ──────────────────────────────────────────────────

#[tokio::test]
async fn self_service_profile_password_forwarding_and_voicemail() {
    let app = require_app!();
    let user_id = app.create_user_with_role("selfie", "end_user").await;

    // Give the user a number, a line binding, and a mailbox (admin side).
    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550002222", "tenant_id": app.tenant_a })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
    let binding = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({
                "phone_number_id": number.json["id"],
                "owner_user_id": user_id,
                "binding_type": "user",
                "tenant_id": app.tenant_a,
            })),
        )
        .await;
    assert_eq!(binding.status, StatusCode::CREATED, "{}", binding.text);
    let binding_id = binding.json["id"].as_str().unwrap().to_string();
    let mailbox = app
        .admin(
            Method::POST,
            "/api/v1/voicemail/mailboxes",
            Some(json!({
                "tenant_id": app.tenant_a,
                "user_id": user_id,
                "line_binding_id": binding_id,
                "pin": "1234",
            })),
        )
        .await;
    assert_eq!(mailbox.status, StatusCode::CREATED, "{}", mailbox.text);

    // The static platform token has no user behind it, so /me is closed to it.
    let no_user = app.admin(Method::GET, "/api/v1/me/profile", None).await;
    assert_eq!(no_user.status, StatusCode::UNAUTHORIZED, "{}", no_user.text);

    let jwt = app.login("selfie", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();

    let profile = app
        .send_as(&jwt, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(profile.status, StatusCode::OK, "{}", profile.text);
    assert_eq!(profile.json["username"], "selfie");
    assert_eq!(profile.json["role"], "end_user");

    let updated = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/profile",
            Some(json!({ "display_name": "Self Service", "timezone": "Europe/Berlin" })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json["display_name"], "Self Service");
    assert_eq!(updated.json["timezone"], "Europe/Berlin");

    let forwarding = app
        .send_as(&jwt, Method::GET, "/api/v1/me/forwarding", None)
        .await;
    assert_eq!(forwarding.status, StatusCode::OK, "{}", forwarding.text);
    assert_eq!(
        forwarding.json.as_array().unwrap().len(),
        1,
        "{}",
        forwarding.text
    );
    assert_eq!(forwarding.json[0]["phone_number"], "+15550002222");
    let fwd_id = forwarding.json[0]["id"].as_str().unwrap().to_string();
    let set_fwd = app
        .send_as(
            &jwt,
            Method::PATCH,
            &format!("/api/v1/me/forwarding/{fwd_id}"),
            Some(json!({ "forward_on_no_answer": "+15559990000", "no_answer_timeout_secs": 15 })),
        )
        .await;
    assert_eq!(set_fwd.status, StatusCode::OK, "{}", set_fwd.text);
    assert_eq!(set_fwd.json["forward_on_no_answer"], "+15559990000");
    assert_eq!(set_fwd.json["no_answer_timeout_secs"], 15);

    let vm = app
        .send_as(&jwt, Method::GET, "/api/v1/me/voicemail", None)
        .await;
    assert_eq!(vm.status, StatusCode::OK, "{}", vm.text);
    assert_eq!(vm.json["enabled"], true);
    let vm_stats = app
        .send_as(&jwt, Method::GET, "/api/v1/me/voicemail/stats", None)
        .await;
    assert_eq!(vm_stats.status, StatusCode::OK, "{}", vm_stats.text);
    let messages = app
        .send_as(&jwt, Method::GET, "/api/v1/me/voicemail/messages", None)
        .await;
    assert_eq!(messages.status, StatusCode::OK, "{}", messages.text);

    // Password change needs the current password and takes effect for login.
    let wrong = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/password",
            Some(json!({ "current_password": "nope", "new_password": "N3w-Passw0rd!" })),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.text);
    let changed = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/password",
            Some(json!({ "current_password": PASSWORD, "new_password": "N3w-Passw0rd!" })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.text);
    assert_eq!(
        app.login("selfie", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.login("selfie", "N3w-Passw0rd!").await.status,
        StatusCode::OK
    );

    app.finish().await;
}

// ─── WebSocket event stream ──────────────────────────────────────────────

#[tokio::test]
async fn websocket_events_require_first_message_auth() {
    use tokio_tungstenite::tungstenite::Message;

    let app = require_app!();
    let port = app.serve().await;
    let url = format!("ws://127.0.0.1:{port}/api/v1/events");

    // Wrong token: auth_error, then the server closes.
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");
    ws.send(Message::Text(
        json!({ "type": "auth", "token": "not-the-token" }).to_string(),
    ))
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("reply in time")
        .expect("frame")
        .expect("ok frame");
    let reply: Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(reply["type"], "auth_error", "{reply}");

    // A non-auth first message is rejected the same way.
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");
    ws.send(Message::Text(json!({ "type": "hello" }).to_string()))
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply: Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(reply["type"], "auth_error", "{reply}");

    // Right token: auth_ok, then published events stream through.
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");
    ws.send(Message::Text(
        json!({ "type": "auth", "token": ADMIN_TOKEN }).to_string(),
    ))
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply: Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    assert_eq!(reply["type"], "auth_ok", "{reply}");

    // Give the subscription a moment to attach, then publish.
    tokio::time::sleep(Duration::from_millis(100)).await;
    app.event_bus
        .publish(PlatformEvent::System(SystemEvent::ConfigChanged {
            entity_type: "device".to_string(),
            entity_id: "device-ws-test".to_string(),
            tenant_id: app.tenant_a,
            timestamp: SystemTime::now(),
        }))
        .await
        .expect("publish");
    // The server announces the subscription first, then streams events.
    let mut frames = Vec::new();
    for _ in 0..3 {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("frame in time")
            .unwrap()
            .unwrap();
        let text = frame.to_text().unwrap().to_string();
        let done = text.contains("device-ws-test");
        frames.push(text);
        if done {
            break;
        }
    }
    let first: Value = serde_json::from_str(&frames[0]).unwrap();
    assert_eq!(first["type"], "connected", "{}", frames[0]);
    assert!(
        frames.iter().any(|f| f.contains("device-ws-test")),
        "event never arrived; frames: {frames:?}"
    );

    app.finish().await;
}

// ─── Multipart uploads ───────────────────────────────────────────────────

/// A small but well-formed 8 kHz mono 16-bit PCM WAV file.
fn wav_bytes(samples: usize) -> Vec<u8> {
    let data_len = (samples * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
    out.extend_from_slice(&16000u32.to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..samples {
        out.extend_from_slice(&((i % 64) as i16 * 100).to_le_bytes());
    }
    out
}

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

#[tokio::test]
async fn avatar_upload_download_and_delete() {
    let app = require_app!();
    app.create_user_with_role("selfie", "end_user").await;
    let jwt = app.login("selfie", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let path = "/api/v1/me/profile/avatar";

    let none = app.send_as(&jwt, Method::GET, path, None).await;
    assert_eq!(none.status, StatusCode::NOT_FOUND, "{}", none.text);

    // Unknown field name (both `avatar` and `file` are accepted), unsupported
    // type, empty file, oversize: all 400.
    let wrong_field = app
        .send_multipart(
            &jwt,
            path,
            &[("picture", Some(("a.png", "image/png")), PNG_MAGIC)],
        )
        .await;
    assert_eq!(
        wrong_field.status,
        StatusCode::BAD_REQUEST,
        "{}",
        wrong_field.text
    );
    let text_file = app
        .send_multipart(
            &jwt,
            path,
            &[("avatar", Some(("a.txt", "text/plain")), b"hello")],
        )
        .await;
    assert_eq!(
        text_file.status,
        StatusCode::BAD_REQUEST,
        "{}",
        text_file.text
    );
    let empty = app
        .send_multipart(&jwt, path, &[("avatar", Some(("a.png", "image/png")), b"")])
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);
    let huge = vec![0u8; 2 * 1024 * 1024 + 1];
    let oversize = app
        .send_multipart(
            &jwt,
            path,
            &[("avatar", Some(("a.png", "image/png")), &huge)],
        )
        .await;
    assert_eq!(
        oversize.status,
        StatusCode::BAD_REQUEST,
        "{}",
        oversize.text
    );

    // A PNG round-trips byte for byte with the right content type.
    let mut png = PNG_MAGIC.to_vec();
    png.extend_from_slice(&[7u8; 512]);
    let uploaded = app
        .send_multipart(
            &jwt,
            path,
            &[("avatar", Some(("me.png", "image/png")), &png)],
        )
        .await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.text);
    let got = app.send_as(&jwt, Method::GET, path, None).await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.header("content-type"), Some("image/png"));
    assert_eq!(got.bytes, png);
    let profile = app
        .send_as(&jwt, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(
        profile.json["has_profile_picture"], true,
        "{}",
        profile.text
    );

    // Replacing with a JPEG swaps the stored file and its type.
    let jpeg = vec![0xFFu8, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];
    let replaced = app
        .send_multipart(
            &jwt,
            path,
            &[("avatar", Some(("me.jpg", "image/jpeg")), &jpeg)],
        )
        .await;
    assert_eq!(replaced.status, StatusCode::CREATED, "{}", replaced.text);
    let got = app.send_as(&jwt, Method::GET, path, None).await;
    assert_eq!(got.header("content-type"), Some("image/jpeg"));
    assert_eq!(got.bytes, jpeg);

    let deleted = app.send_as(&jwt, Method::DELETE, path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    assert_eq!(
        app.send_as(&jwt, Method::GET, path, None).await.status,
        StatusCode::NOT_FOUND
    );
    let profile = app
        .send_as(&jwt, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(
        profile.json["has_profile_picture"], false,
        "{}",
        profile.text
    );

    app.finish().await;
}

#[tokio::test]
async fn voicemail_greetings_admin_and_self_service() {
    let app = require_app!();
    let user_id = app.create_user_with_role("greeter", "end_user").await;
    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550003333", "tenant_id": app.tenant_a })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
    let binding = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({
                "phone_number_id": number.json["id"],
                "owner_user_id": user_id,
                "binding_type": "user",
                "tenant_id": app.tenant_a,
            })),
        )
        .await;
    assert_eq!(binding.status, StatusCode::CREATED, "{}", binding.text);
    let mailbox = app
        .admin(
            Method::POST,
            "/api/v1/voicemail/mailboxes",
            Some(json!({
                "tenant_id": app.tenant_a,
                "user_id": user_id,
                "line_binding_id": binding.json["id"],
                "pin": "1234",
            })),
        )
        .await;
    assert_eq!(mailbox.status, StatusCode::CREATED, "{}", mailbox.text);
    let mailbox_id = mailbox.json["id"].as_str().unwrap().to_string();
    let admin_path = format!("/api/v1/voicemail/mailboxes/{mailbox_id}/greetings");
    let busy_wav = wav_bytes(800);

    // Admin upload: validation first.
    let missing_type = app
        .send_multipart(
            ADMIN_TOKEN,
            &admin_path,
            &[("audio", Some(("g.wav", "audio/wav")), &busy_wav)],
        )
        .await;
    assert_eq!(
        missing_type.status,
        StatusCode::BAD_REQUEST,
        "{}",
        missing_type.text
    );
    let bad_type = app
        .send_multipart(
            ADMIN_TOKEN,
            &admin_path,
            &[
                ("greeting_type", None, b"holiday"),
                ("audio", Some(("g.wav", "audio/wav")), &busy_wav),
            ],
        )
        .await;
    assert_eq!(
        bad_type.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_type.text
    );
    let empty = app
        .send_multipart(
            ADMIN_TOKEN,
            &admin_path,
            &[
                ("greeting_type", None, b"busy"),
                ("audio", Some(("g.wav", "audio/wav")), b""),
            ],
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);

    let uploaded = app
        .send_multipart(
            ADMIN_TOKEN,
            &admin_path,
            &[
                ("greeting_type", None, b"busy"),
                ("audio", Some(("g.wav", "audio/wav")), &busy_wav),
            ],
        )
        .await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", uploaded.text);
    assert_eq!(uploaded.json["greeting_type"], "busy");

    let list = app.admin(Method::GET, &admin_path, None).await;
    assert_eq!(list.json.as_array().unwrap().len(), 1, "{}", list.text);
    let activated = app
        .admin(
            Method::PUT,
            &format!("{admin_path}/busy/active"),
            Some(json!({ "active": true })),
        )
        .await;
    assert_eq!(activated.status, StatusCode::OK, "{}", activated.text);
    let audio = app
        .admin(Method::GET, &format!("{admin_path}/busy/audio"), None)
        .await;
    assert_eq!(audio.status, StatusCode::OK, "{}", audio.text);
    assert_eq!(audio.header("content-type"), Some("audio/wav"));
    assert_eq!(audio.bytes, busy_wav);

    // Another tenant cannot touch the mailbox.
    let cross = app
        .admin_in(app.tenant_b, Method::GET, &admin_path, None)
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    // The owner manages greetings through /me with the same files.
    let jwt = app.login("greeter", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let me_path = "/api/v1/me/voicemail/greetings";
    let temp_wav = wav_bytes(400);
    let temp = app
        .send_multipart(
            &jwt,
            me_path,
            &[
                ("greeting_type", None, b"temporary"),
                ("expires_in", None, b"24"),
                ("audio", Some(("t.wav", "audio/wav")), &temp_wav),
            ],
        )
        .await;
    assert_eq!(temp.status, StatusCode::CREATED, "{}", temp.text);
    assert_eq!(temp.json["greeting_type"], "temporary");
    assert!(!temp.json["expires_at"].is_null(), "{}", temp.text);
    let bad_expiry = app
        .send_multipart(
            &jwt,
            me_path,
            &[
                ("greeting_type", None, b"name"),
                ("expires_in", None, b"soon"),
                ("audio", Some(("n.wav", "audio/wav")), &temp_wav),
            ],
        )
        .await;
    assert_eq!(
        bad_expiry.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_expiry.text
    );

    let mine = app.send_as(&jwt, Method::GET, me_path, None).await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.text);
    assert_eq!(mine.json.as_array().unwrap().len(), 2, "{}", mine.text);
    let busy_audio = app
        .send_as(&jwt, Method::GET, &format!("{me_path}/busy/audio"), None)
        .await;
    assert_eq!(busy_audio.status, StatusCode::OK, "{}", busy_audio.text);
    assert_eq!(busy_audio.bytes, busy_wav);
    let temp_audio = app
        .send_as(
            &jwt,
            Method::GET,
            &format!("{me_path}/temporary/audio"),
            None,
        )
        .await;
    assert_eq!(temp_audio.bytes, temp_wav);

    let del_temp = app
        .send_as(&jwt, Method::DELETE, &format!("{me_path}/temporary"), None)
        .await;
    assert_eq!(del_temp.status, StatusCode::NO_CONTENT, "{}", del_temp.text);
    let del_busy = app
        .admin(Method::DELETE, &format!("{admin_path}/busy"), None)
        .await;
    assert_eq!(del_busy.status, StatusCode::NO_CONTENT, "{}", del_busy.text);
    let none = app.admin(Method::GET, &admin_path, None).await;
    assert!(none.json.as_array().unwrap().is_empty(), "{}", none.text);
    let gone = app
        .send_as(&jwt, Method::GET, &format!("{me_path}/busy/audio"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);

    app.finish().await;
}

#[tokio::test]
async fn prompts_crud_and_audio_upload() {
    let app = require_app!();

    let no_text = app
        .admin(
            Method::POST,
            "/api/v1/prompts",
            Some(json!({ "name": "welcome", "source_type": "tts" })),
        )
        .await;
    assert_eq!(
        no_text.status,
        StatusCode::BAD_REQUEST,
        "tts prompts need text: {}",
        no_text.text
    );

    let created = app
        .admin(
            Method::POST,
            "/api/v1/prompts",
            Some(json!({
                "name": "welcome",
                "description": "Main menu welcome",
                "category": "ivr",
                "source_type": "file",
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let id = created.json["id"].as_str().unwrap().to_string();
    assert_eq!(created.json["category"], "ivr");
    assert_eq!(created.json["source_type"], "file");
    assert_eq!(created.json["has_audio"], false);
    assert_eq!(created.json["tenant_id"], app.tenant_a.to_string());
    let audio_path = format!("/api/v1/prompts/{id}/audio");

    let no_audio = app.admin(Method::GET, &audio_path, None).await;
    assert_eq!(no_audio.status, StatusCode::NOT_FOUND, "{}", no_audio.text);

    let wrong_field = app
        .send_multipart(
            ADMIN_TOKEN,
            &audio_path,
            &[("file", Some(("w.wav", "audio/wav")), &wav_bytes(10))],
        )
        .await;
    assert_eq!(
        wrong_field.status,
        StatusCode::BAD_REQUEST,
        "{}",
        wrong_field.text
    );
    let empty = app
        .send_multipart(
            ADMIN_TOKEN,
            &audio_path,
            &[("audio", Some(("w.wav", "audio/wav")), b"")],
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);

    let wav = wav_bytes(1600);
    let uploaded = app
        .send_multipart(
            ADMIN_TOKEN,
            &audio_path,
            &[("audio", Some(("welcome.wav", "audio/wav")), &wav)],
        )
        .await;
    assert_eq!(uploaded.status, StatusCode::OK, "{}", uploaded.text);
    assert_eq!(uploaded.json["has_audio"], true);
    assert_eq!(uploaded.json["audio_format"], "wav");
    assert_eq!(uploaded.json["file_size"], wav.len());

    let download = app.admin(Method::GET, &audio_path, None).await;
    assert_eq!(download.status, StatusCode::OK, "{}", download.text);
    assert_eq!(download.header("content-type"), Some("audio/wav"));
    assert_eq!(download.bytes, wav);

    let listed = app
        .admin(Method::GET, "/api/v1/prompts?category=ivr", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json.as_array().unwrap().len(), 1, "{}", listed.text);
    let other_category = app
        .admin(Method::GET, "/api/v1/prompts?category=moh", None)
        .await;
    assert!(
        other_category.json.as_array().unwrap().is_empty(),
        "{}",
        other_category.text
    );

    let cross = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/prompts/{id}"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);
    let cross_audio = app
        .admin_in(app.tenant_b, Method::GET, &audio_path, None)
        .await;
    assert_eq!(
        cross_audio.status,
        StatusCode::NOT_FOUND,
        "{}",
        cross_audio.text
    );

    let deleted = app
        .admin(Method::DELETE, &format!("/api/v1/prompts/{id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    assert_eq!(
        app.admin(Method::GET, &format!("/api/v1/prompts/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    app.finish().await;
}
