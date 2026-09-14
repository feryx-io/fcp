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
use fcp_core::sip_trace::{SipTraceMessage, SipTraceStore};
use fcp_core::storage::{CallDetailRecord, CallDisposition, CdrStore, DeviceStore, HangupCause};
use fcp_core::storage::{Registration, RegistrationStore};
use fcp_core::{ApiConfig, CallId, DynStorage, Tenant, TenantId, TenantStore, UserId};
use fcp_events::InMemoryEventBus;
use fcp_events::{EventPublisher, PlatformEvent, SystemEvent};
use fcp_flow::FlowStore;
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::prelude::{parse_request, parse_response, Dialog, Subscription};
use fcp_sip::prelude::{
    DialogManager, SubscriptionManager, TransactionManager, TransportContext, TransportDispatcher,
};
use fcp_sip::Call;
use fcp_sip::{B2buaConfig, CallManager};
use fcp_storage_postgres::api_token::PostgresApiTokenStore;
use fcp_storage_postgres::compliance::PostgresComplianceStore;
use fcp_storage_postgres::emergency::PostgresEmergencyStore;
use fcp_storage_postgres::flow_trace::PostgresFlowTraceStore;
use fcp_storage_postgres::prompt::PostgresPromptStore;
use fcp_storage_postgres::recording::PostgresRecordingStore;
use fcp_storage_postgres::sip_trace::PostgresSipTraceStore;
use fcp_storage_postgres::{PostgresConfig, PostgresStorage};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::{Connection, PgConnection};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tower::ServiceExt;
use uuid::Uuid;

const ADMIN_TOKEN: &str = "test-admin-token";
const CONFERENCE_TOKEN: &str = "test-conference-server-token";
const PROVISIONING_TOKEN: &str = "test-provisioning-shared-token";
const REALM: &str = "pbx.example.com";

// ─── Harness ─────────────────────────────────────────────────────────────

/// Skip the test (returning early) unless a database is configured.
macro_rules! require_app {
    () => {
        require_app!(DeploymentMode::default())
    };
    ($mode:expr) => {
        match App::build($mode, None).await {
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
    sip_traces: Arc<PostgresSipTraceStore>,
    emergency: Arc<PostgresEmergencyStore>,
    compliance: Arc<PostgresComplianceStore>,
    event_bus: Arc<InMemoryEventBus>,
    dialog_manager: Arc<DialogManager>,
    subscription_manager: Arc<SubscriptionManager>,
    call_manager: Arc<CallManager>,
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
    /// An app whose live conference endpoints proxy to `conference_url`.
    async fn with_conference_server(conference_url: &str) -> Option<Self> {
        Self::build(DeploymentMode::default(), Some(conference_url.to_string())).await
    }

    async fn build(mode: DeploymentMode, conference_url: Option<String>) -> Option<Self> {
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
        let sip_traces = Arc::new(PostgresSipTraceStore::new(storage.pool().clone()));
        let emergency = Arc::new(PostgresEmergencyStore::new(storage.pool().clone()));
        let compliance = Arc::new(PostgresComplianceStore::new(storage.pool().clone()));
        let dyn_storage = Arc::new(
            DynStorage::from_all_in_one(Arc::clone(&storage))
                .with_api_token_store(api_tokens)
                .with_flow_trace_store(Arc::clone(&traces) as Arc<dyn FlowTraceStore>)
                .with_sip_trace_store(Arc::clone(&sip_traces) as Arc<dyn SipTraceStore>)
                .with_emergency_store(
                    Arc::clone(&emergency) as Arc<dyn fcp_core::e911::EmergencyStore>
                )
                .with_compliance_store(
                    Arc::clone(&compliance) as Arc<dyn fcp_core::compliance::ComplianceStore>
                )
                .with_prompt_store(Arc::new(PostgresPromptStore::new(storage.pool().clone()))
                    as Arc<dyn fcp_core::prompt::PromptStore>)
                .with_recording_store(
                    Arc::new(PostgresRecordingStore::new(storage.pool().clone()))
                        as Arc<dyn fcp_core::recording::RecordingStore>,
                ),
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
            Arc::clone(&dialog_manager),
            Arc::clone(&subscription_manager),
            Arc::clone(&call_manager),
            Arc::new(handler),
            None,
            Arc::clone(&event_bus) as Arc<dyn fcp_events::EventBus>,
            Some(queue_manager),
            Some(presence_manager),
            REALM.to_string(),
            tenant_a,
        )
        .with_deployment_mode(mode)
        .with_conference_server_url(conference_url)
        .with_conference_server_auth_token(Some(CONFERENCE_TOKEN.to_string()))
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
            sip_traces,
            emergency,
            compliance,
            event_bus,
            dialog_manager,
            subscription_manager,
            call_manager,
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
        self.seed_cdr_full(tenant, answered_ms, cost).await.0
    }

    /// Like [`seed_cdr`] but also returns the SIP Call-ID it recorded.
    async fn seed_cdr_full(
        &self,
        tenant: TenantId,
        answered_ms: Option<u64>,
        cost: Option<f64>,
    ) -> (CallId, String) {
        self.seed_cdr_aged(tenant, answered_ms, cost, Duration::from_secs(300))
            .await
    }

    /// Like [`seed_cdr_full`] with the call starting `age` ago.
    async fn seed_cdr_aged(
        &self,
        tenant: TenantId,
        answered_ms: Option<u64>,
        cost: Option<f64>,
        age: Duration,
    ) -> (CallId, String) {
        let start = SystemTime::now() - age;
        let (disposition, answer_time) = match answered_ms {
            Some(_) => (
                CallDisposition::Answered,
                Some(start + Duration::from_secs(2)),
            ),
            None => (CallDisposition::NoAnswer, None),
        };
        let cdr = CallDetailRecord {
            cdr_id: uuid::Uuid::new_v4(),
            kind: fcp_core::storage::CdrKind::Call,
            schema_version: fcp_core::storage::CDR_SCHEMA_VERSION,
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
            direction: None,
            node_id: None,
            region: None,
            correlation_id: None,
            trunk_name: None,
            rtp_packets_lost: None,
            rtp_jitter_ms: None,
            rate_plan_id: None,
            matched_prefix: None,
            billable_seconds: None,
            rated_at: None,
            created_at: None,
            custom_data: None,
        };
        let id = cdr.call_id;
        let sip_call_id = cdr.sip_call_id.clone();
        self.storage.insert_cdr(cdr).await.expect("insert cdr");
        (id, sip_call_id)
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
        self.seed_trace_at(call_id, flow_id, event, SystemTime::now())
            .await
    }

    async fn seed_trace_at(
        &self,
        call_id: &CallId,
        flow_id: &str,
        event: FlowTraceEventType,
        timestamp: SystemTime,
    ) {
        self.traces
            .insert_flow_trace(&FlowTraceEntry {
                id: 0,
                call_id: call_id.to_string(),
                flow_id: flow_id.to_string(),
                flow_name: Some("Main IVR".to_string()),
                node_id: Some("answer".to_string()),
                event_type: event,
                timestamp,
                detail: None,
                flow_version: Some(1),
                elapsed_ms: Some(5),
            })
            .await
            .expect("insert trace");
    }

    /// Insert one SIP trace row the way the B2BUA writes it.
    async fn seed_sip_trace(
        &self,
        call_id: &str,
        sip_call_id: &str,
        tenant: TenantId,
        direction: &str,
        method_or_status: &str,
    ) {
        self.sip_traces
            .insert_sip_trace(&SipTraceMessage {
                id: 0,
                call_id: call_id.to_string(),
                sip_call_id: sip_call_id.to_string(),
                tenant_id: tenant.to_string(),
                timestamp: SystemTime::now(),
                direction: direction.to_string(),
                method_or_status: method_or_status.to_string(),
                cseq: "1 INVITE".to_string(),
                from_uri: "sip:alice@pbx.example.com".to_string(),
                to_uri: "sip:bob@pbx.example.com".to_string(),
                source_addr: "10.0.0.5:5060".to_string(),
                dest_addr: "10.0.0.1:5060".to_string(),
                transport: "udp".to_string(),
                raw_message: format!("{method_or_status} sip:bob@pbx.example.com SIP/2.0\r\n"),
                node_id: Some("node-test".to_string()),
            })
            .await
            .expect("insert sip trace");
    }

    async fn seed_sip_trace_at(
        &self,
        call_id: &str,
        sip_call_id: &str,
        tenant: TenantId,
        direction: &str,
        method_or_status: &str,
        timestamp: SystemTime,
    ) {
        self.sip_traces
            .insert_sip_trace(&SipTraceMessage {
                id: 0,
                call_id: call_id.to_string(),
                sip_call_id: sip_call_id.to_string(),
                tenant_id: tenant.to_string(),
                timestamp,
                direction: direction.to_string(),
                method_or_status: method_or_status.to_string(),
                cseq: "1 INVITE".to_string(),
                from_uri: "sip:alice@pbx.example.com".to_string(),
                to_uri: "sip:bob@pbx.example.com".to_string(),
                source_addr: "10.0.0.5:5060".to_string(),
                dest_addr: "10.0.0.1:5060".to_string(),
                transport: "udp".to_string(),
                raw_message: format!("{method_or_status} sip:bob@pbx.example.com SIP/2.0\r\n"),
                node_id: Some("node-test".to_string()),
            })
            .await
            .expect("insert sip trace");
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
    // The pin must not come back — as a field or as any value. Not a
    // substring search over the whole body: four digits turn up inside a
    // uuid or a timestamp often enough to fail a sound response (CI, PR
    // #57: `"created_at":1788890554.5432115` contains "4321").
    assert!(
        mailbox.json.get("pin").is_none(),
        "pin must not be echoed: {}",
        mailbox.text
    );
    assert!(
        !json_has_value(&mailbox.json, "4321"),
        "pin must not be echoed as any value: {}",
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

// ─── SIP traces ──────────────────────────────────────────────────────────

#[tokio::test]
async fn sip_traces_by_call_id_sip_call_id_and_cdr_fallback() {
    let app = require_app!();

    // A live call: traces exist, no CDR yet. Stored with the canonical id.
    let live = CallId::new();
    let live_sip = format!("{}@10.0.0.5", Uuid::new_v4());
    app.seed_sip_trace(
        &live.to_string(),
        &live_sip,
        app.tenant_a,
        "inbound",
        "INVITE",
    )
    .await;
    app.seed_sip_trace(
        &live.to_string(),
        &live_sip,
        app.tenant_a,
        "outbound",
        "200",
    )
    .await;

    let by_id = app
        .admin(
            Method::GET,
            &format!("/api/v1/calls/{live}/sip-messages"),
            None,
        )
        .await;
    assert_eq!(by_id.status, StatusCode::OK, "{}", by_id.text);
    assert_eq!(
        by_id.json["messages"].as_array().unwrap().len(),
        2,
        "{}",
        by_id.text
    );
    assert_eq!(by_id.json["messages"][0]["method_or_status"], "INVITE");
    assert_eq!(by_id.json["messages"][0]["direction"], "inbound");
    // The bare uuid works as well.
    let bare = live.to_string().trim_start_matches("call-").to_string();
    let by_bare = app
        .admin(
            Method::GET,
            &format!("/api/v1/calls/{bare}/sip-messages"),
            None,
        )
        .await;
    assert_eq!(
        by_bare.json["messages"].as_array().unwrap().len(),
        2,
        "{}",
        by_bare.text
    );
    assert_eq!(by_bare.json["call_id"], bare);

    // Direct SIP Call-ID lookup.
    let by_sip = app
        .admin(
            Method::GET,
            &format!("/api/v1/calls/{live}/sip-messages?sip_call_id={live_sip}"),
            None,
        )
        .await;
    assert_eq!(
        by_sip.json["messages"].as_array().unwrap().len(),
        2,
        "{}",
        by_sip.text
    );

    // A finished call whose traces were keyed only by SIP Call-ID: found via its CDR.
    let (ended, ended_sip) = app.seed_cdr_full(app.tenant_a, Some(5_000), None).await;
    app.seed_sip_trace("", &ended_sip, app.tenant_a, "inbound", "BYE")
        .await;
    let via_cdr = app
        .admin(
            Method::GET,
            &format!("/api/v1/calls/{ended}/sip-messages"),
            None,
        )
        .await;
    assert_eq!(via_cdr.status, StatusCode::OK, "{}", via_cdr.text);
    assert_eq!(
        via_cdr.json["messages"].as_array().unwrap().len(),
        1,
        "{}",
        via_cdr.text
    );
    assert_eq!(via_cdr.json["messages"][0]["method_or_status"], "BYE");

    // Unknown calls are an empty list, and tenant scoping filters rows.
    let unknown = app
        .admin(
            Method::GET,
            &format!("/api/v1/calls/{}/sip-messages", CallId::new()),
            None,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::OK, "{}", unknown.text);
    assert!(unknown.json["messages"].as_array().unwrap().is_empty());

    let b_call = CallId::new();
    app.seed_sip_trace(
        &b_call.to_string(),
        "b@10.0.1.5",
        app.tenant_b,
        "inbound",
        "INVITE",
    )
    .await;
    let scoped_a = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/calls/{b_call}/sip-messages"),
            None,
        )
        .await;
    assert!(
        scoped_a.json["messages"].as_array().unwrap().is_empty(),
        "{}",
        scoped_a.text
    );
    let scoped_b = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/calls/{b_call}/sip-messages"),
            None,
        )
        .await;
    assert_eq!(
        scoped_b.json["messages"].as_array().unwrap().len(),
        1,
        "{}",
        scoped_b.text
    );
    let platform = app
        .admin(
            Method::GET,
            &format!("/api/v1/calls/{b_call}/sip-messages"),
            None,
        )
        .await;
    assert_eq!(
        platform.json["messages"].as_array().unwrap().len(),
        1,
        "{}",
        platform.text
    );

    app.finish().await;
}

// ─── Debug endpoints ─────────────────────────────────────────────────────

fn sip_dialog(sip_call_id: &str) -> Dialog {
    let invite = format!(
        "INVITE sip:bob@{REALM} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKtest1\r\n\
         From: <sip:alice@{REALM}>;tag=alice-tag\r\n\
         To: <sip:bob@{REALM}>\r\n\
         Call-ID: {sip_call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:alice@10.0.0.5:5060>\r\n\
         Max-Forwards: 70\r\n\
         Content-Length: 0\r\n\r\n"
    );
    let ok = format!(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKtest1\r\n\
         From: <sip:alice@{REALM}>;tag=alice-tag\r\n\
         To: <sip:bob@{REALM}>;tag=bob-tag\r\n\
         Call-ID: {sip_call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Contact: <sip:bob@10.0.0.6:5060>\r\n\
         Content-Length: 0\r\n\r\n"
    );
    let req = parse_request(&Bytes::from(invite)).expect("parse INVITE");
    let resp = parse_response(&Bytes::from(ok)).expect("parse 200");
    Dialog::new_uas(
        &req,
        &resp,
        fcp_sip::prelude::SipUri::parse(&format!("sip:bob@{REALM}")).unwrap(),
        fcp_sip::prelude::SipUri::parse(&format!("sip:alice@{REALM}")).unwrap(),
    )
    .expect("dialog from INVITE/200")
}

fn sip_subscription(sip_call_id: &str) -> Subscription {
    let subscribe = format!(
        "SUBSCRIBE sip:bob@{REALM} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKsub1\r\n\
         From: <sip:alice@{REALM}>;tag=sub-from\r\n\
         To: <sip:bob@{REALM}>\r\n\
         Call-ID: {sip_call_id}\r\n\
         CSeq: 1 SUBSCRIBE\r\n\
         Event: presence\r\n\
         Expires: 3600\r\n\
         Contact: <sip:alice@10.0.0.5:5060>\r\n\
         Content-Length: 0\r\n\r\n"
    );
    let ok = format!(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP 10.0.0.5:5060;branch=z9hG4bKsub1\r\n\
         From: <sip:alice@{REALM}>;tag=sub-from\r\n\
         To: <sip:bob@{REALM}>;tag=sub-to\r\n\
         Call-ID: {sip_call_id}\r\n\
         CSeq: 1 SUBSCRIBE\r\n\
         Event: presence\r\n\
         Expires: 3600\r\n\
         Contact: <sip:bob@10.0.0.6:5060>\r\n\
         Content-Length: 0\r\n\r\n"
    );
    let req = parse_request(&Bytes::from(subscribe)).expect("parse SUBSCRIBE");
    let resp = parse_response(&Bytes::from(ok)).expect("parse 200");
    Subscription::new_notifier(
        &req,
        &resp,
        fcp_sip::prelude::SipUri::parse(&format!("sip:bob@{REALM}")).unwrap(),
        fcp_sip::prelude::SipUri::parse(&format!("sip:alice@{REALM}")).unwrap(),
    )
    .expect("subscription from SUBSCRIBE/200")
}

#[tokio::test]
async fn debug_endpoints_reflect_runtime_state_per_tenant() {
    let app = require_app!();

    // Empty runtime first.
    let empty = app.admin(Method::GET, "/api/v1/debug/summary", None).await;
    assert_eq!(empty.status, StatusCode::OK, "{}", empty.text);
    assert_eq!(empty.json["active_dialogs"], 0, "{}", empty.text);
    assert_eq!(empty.json["active_calls"], 0);
    let limits = app
        .admin(Method::GET, "/api/v1/debug/rate-limits", None)
        .await;
    assert_eq!(limits.status, StatusCode::OK, "{}", limits.text);
    assert!(limits.json.is_array(), "{}", limits.text);

    // One confirmed dialog attached to a tenant-A call, one subscription.
    let sip_call_id = format!("{}@10.0.0.5", Uuid::new_v4());
    let dialog = sip_dialog(&sip_call_id);
    app.dialog_manager
        .insert(dialog.clone())
        .expect("insert dialog");
    let mut call = Call::new_originated(
        format!("sip:alice@{REALM}"),
        Some("Alice".to_string()),
        format!("sip:bob@{REALM}"),
        app.tenant_a,
    );
    call.a_leg.dialog = Some(dialog);
    let internal_id = app.call_manager.create_call(call);
    let sub_call_id = format!("{}@10.0.0.5", Uuid::new_v4());
    app.subscription_manager
        .insert(sip_subscription(&sub_call_id))
        .expect("insert subscription");

    // Platform view sees everything.
    let summary = app.admin(Method::GET, "/api/v1/debug/summary", None).await;
    assert_eq!(summary.json["active_dialogs"], 1, "{}", summary.text);
    assert_eq!(summary.json["active_subscriptions"], 1, "{}", summary.text);
    assert_eq!(summary.json["active_calls"], 1, "{}", summary.text);
    let dialogs = app.admin(Method::GET, "/api/v1/debug/dialogs", None).await;
    assert_eq!(
        dialogs.json.as_array().unwrap().len(),
        1,
        "{}",
        dialogs.text
    );
    assert_eq!(dialogs.json[0]["call_id"], sip_call_id);
    assert_eq!(dialogs.json[0]["state"], "Confirmed");
    assert_eq!(dialogs.json[0]["local_tag"], "bob-tag");
    assert_eq!(dialogs.json[0]["remote_tag"], "alice-tag");
    let subs = app
        .admin(Method::GET, "/api/v1/debug/subscriptions", None)
        .await;
    assert_eq!(subs.json.as_array().unwrap().len(), 1, "{}", subs.text);
    assert_eq!(subs.json[0]["event"], "presence");
    let calls = app.admin(Method::GET, "/api/v1/debug/calls", None).await;
    assert_eq!(calls.json.as_array().unwrap().len(), 1, "{}", calls.text);
    assert_eq!(calls.json[0]["call_id"], internal_id.to_string());

    // A tenant-A admin sees its own call and the dialog behind it.
    app.create_user_with_role("dbg-a", "tenant_admin").await;
    let jwt_a = app.login("dbg-a", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let a_calls = app
        .send_as(&jwt_a, Method::GET, "/api/v1/debug/calls", None)
        .await;
    assert_eq!(a_calls.status, StatusCode::OK, "{}", a_calls.text);
    assert_eq!(
        a_calls.json.as_array().unwrap().len(),
        1,
        "{}",
        a_calls.text
    );
    let a_dialogs = app
        .send_as(&jwt_a, Method::GET, "/api/v1/debug/dialogs", None)
        .await;
    assert_eq!(
        a_dialogs.json.as_array().unwrap().len(),
        1,
        "{}",
        a_dialogs.text
    );
    let a_summary = app
        .send_as(&jwt_a, Method::GET, "/api/v1/debug/summary", None)
        .await;
    assert_eq!(a_summary.json["active_dialogs"], 1, "{}", a_summary.text);
    assert_eq!(a_summary.json["active_calls"], 1, "{}", a_summary.text);

    // A tenant-B admin sees nothing of it.
    let b_user = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/users",
            Some(json!({ "username": "dbg-b", "password": PASSWORD, "role": "tenant_admin" })),
        )
        .await;
    assert_eq!(b_user.status, StatusCode::CREATED, "{}", b_user.text);
    let jwt_b = app.login("dbg-b", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let b_calls = app
        .send_as(&jwt_b, Method::GET, "/api/v1/debug/calls", None)
        .await;
    assert!(
        b_calls.json.as_array().unwrap().is_empty(),
        "{}",
        b_calls.text
    );
    let b_dialogs = app
        .send_as(&jwt_b, Method::GET, "/api/v1/debug/dialogs", None)
        .await;
    assert!(
        b_dialogs.json.as_array().unwrap().is_empty(),
        "{}",
        b_dialogs.text
    );
    let b_summary = app
        .send_as(&jwt_b, Method::GET, "/api/v1/debug/summary", None)
        .await;
    assert_eq!(b_summary.json["active_dialogs"], 0, "{}", b_summary.text);
    assert_eq!(b_summary.json["active_calls"], 0, "{}", b_summary.text);

    // End users are kept out of debug entirely.
    app.create_user_with_role("dbg-eu", "end_user").await;
    let jwt_eu = app.login("dbg-eu", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let denied = app
        .send_as(&jwt_eu, Method::GET, "/api/v1/debug/summary", None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text);

    app.finish().await;
}

// ─── Emergency calling (E911) ────────────────────────────────────────────

#[tokio::test]
async fn e911_config_readiness_validation_and_audit() {
    use fcp_core::e911::{EmergencyCallRecord, EmergencyKind, EmergencyStore};

    let app = require_app!();

    // Defaults until configured.
    let cfg = app
        .admin(Method::GET, "/api/v1/emergency/config", None)
        .await;
    assert_eq!(cfg.status, StatusCode::OK, "{}", cfg.text);
    assert_eq!(cfg.json["configured"], false);
    assert_eq!(cfg.json["emergency_numbers"], json!(["911"]));
    assert_eq!(cfg.json["test_numbers"], json!(["933"]));

    let bad = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({ "emergency_numbers": ["9-1-1"] })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let clash = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({ "emergency_numbers": ["911", "933"] })),
        )
        .await;
    assert_eq!(clash.status, StatusCode::BAD_REQUEST, "{}", clash.text);

    let saved = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({
                "emergency_numbers": ["911", "112", " 911 "],
                "notify_contacts": ["security@example.com", "sip:frontdesk@pbx.example.com"],
                "require_dispatchable_location": true
            })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json["configured"], true);
    assert_eq!(
        saved.json["emergency_numbers"],
        json!(["911", "112"]),
        "deduplicated"
    );
    assert_eq!(
        saved.json["test_numbers"],
        json!(["933"]),
        "untouched fields keep defaults"
    );
    assert_eq!(saved.json["require_dispatchable_location"], true);
    let reread = app
        .admin(Method::GET, "/api/v1/emergency/config", None)
        .await;
    assert_eq!(
        reread.json["notify_contacts"][1],
        "sip:frontdesk@pbx.example.com"
    );
    // Tenant B still sees defaults.
    let other = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/emergency/config", None)
        .await;
    assert_eq!(other.json["configured"], false, "{}", other.text);

    // Readiness: a site with an address and ELIN makes its users ready.
    let site = app
        .admin(
            Method::POST,
            "/api/v1/locations",
            Some(json!({
                "name": "HQ",
                "address_line1": "1 State St",
                "city": "Albany",
                "state": "NY",
                "postal_code": "12207",
                "country": "US",
                "floor": "3",
                "e911_callback_number": "+15185550100"
            })),
        )
        .await;
    assert_eq!(site.status, StatusCode::CREATED, "{}", site.text);
    let site_id = site.json["id"].as_str().unwrap().to_string();
    assert_eq!(site.json["e911_callback_number"], "+15185550100");

    let ready_user = app.create_user("ready", None).await;
    let placed = app
        .admin(
            Method::PUT,
            "/api/v1/users/ready",
            Some(json!({ "location_id": site_id })),
        )
        .await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.text);
    app.create_user("nowhere", None).await;

    let readiness = app
        .admin(Method::GET, "/api/v1/emergency/readiness", None)
        .await;
    assert_eq!(readiness.status, StatusCode::OK, "{}", readiness.text);
    assert_eq!(readiness.json["summary"]["users"], 2, "{}", readiness.text);
    assert_eq!(readiness.json["summary"]["missing_location"], 1);
    let rows = readiness.json["users"].as_array().unwrap();
    let ready = rows.iter().find(|u| u["username"] == "ready").unwrap();
    assert_eq!(ready["location"]["source"], "user_location", "{ready}");
    assert_eq!(ready["location"]["callback_number"], "+15185550100");
    assert_eq!(ready["location"]["address"]["address2"], "Floor 3");
    assert_eq!(ready["location"]["location_id"], site_id);
    // Site addresses are never "validated", so the user is not fully ready.
    assert_eq!(ready["ready"], false, "{ready}");
    assert_eq!(ready["issues"].as_array().unwrap().len(), 1, "{ready}");
    let nowhere = rows.iter().find(|u| u["username"] == "nowhere").unwrap();
    assert_eq!(nowhere["ready"], false);
    assert!(nowhere["location"].is_null());
    let _ = ready_user;

    // Ad-hoc address validation.
    let check = app
        .admin(
            Method::POST,
            "/api/v1/emergency/validate-address",
            Some(json!({ "address": {
                "address1": "State Street", "city": "", "state": "New York",
                "postal_code": "1220", "country": "US", "validated": false
            }})),
        )
        .await;
    assert_eq!(check.status, StatusCode::OK, "{}", check.text);
    assert_eq!(check.json["valid"], false);
    assert_eq!(
        check.json["issues"].as_array().unwrap().len(),
        4,
        "{}",
        check.text
    );
    assert_eq!(check.json["validator"], "basic");

    // Validating a number's E911 address marks it validated when it passes.
    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({
                "e164": "+15185550199",
                "tenant_id": app.tenant_a,
                "e911_address": {
                    "address1": " 2  Main St", "city": "Albany", "state": "ny",
                    "postal_code": "12207", "country": "US", "validated": false
                },
                "e911_callback": "+15185550199"
            })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
    let number_id = number.json["id"].as_str().unwrap().to_string();
    assert_eq!(number.json["e911_address"]["validated"], false);
    let validated = app
        .admin(
            Method::POST,
            &format!("/api/v1/phone-numbers/{number_id}/e911/validate"),
            None,
        )
        .await;
    assert_eq!(validated.status, StatusCode::OK, "{}", validated.text);
    assert_eq!(validated.json["validated"], true);
    assert!(!validated.json["validated_at"].is_null());
    assert_eq!(validated.json["validation"]["normalized"]["state"], "NY");
    let stored = app
        .admin(
            Method::GET,
            &format!("/api/v1/phone-numbers/{number_id}"),
            None,
        )
        .await;
    assert_eq!(
        stored.json["e911_address"]["validated"], true,
        "{}",
        stored.text
    );
    assert_eq!(
        stored.json["e911_address"]["address1"], "2 Main St",
        "normalized on save"
    );
    let cross = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            &format!("/api/v1/phone-numbers/{number_id}/e911/validate"),
            None,
        )
        .await;
    assert_eq!(cross.status, StatusCode::NOT_FOUND, "{}", cross.text);

    // Trunks carry the emergency flag through create and update.
    let trunk = app
        .admin(
            Method::POST,
            "/api/v1/trunks",
            Some(json!({ "name": "e911-trunk", "hosts": ["sip.carrier.example.net"], "emergency_capable": true })),
        )
        .await;
    assert_eq!(trunk.status, StatusCode::CREATED, "{}", trunk.text);
    assert_eq!(trunk.json["emergency_capable"], true);
    let trunk_id = trunk.json["id"].as_str().unwrap().to_string();
    let cleared = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/trunks/{trunk_id}"),
            Some(json!({ "emergency_capable": false })),
        )
        .await;
    assert_eq!(cleared.json["emergency_capable"], false, "{}", cleared.text);

    // Audit trail: records written by the B2BUA are listed per tenant.
    let record = EmergencyCallRecord {
        id: 0,
        tenant_id: app.tenant_a,
        call_id: CallId::new().to_string(),
        sip_call_id: Some("abc@10.0.0.5".to_string()),
        caller_uri: format!("sip:ready@{REALM}"),
        caller_username: Some("ready".to_string()),
        dialed_number: "911".to_string(),
        kind: EmergencyKind::Live,
        callback_number: Some("+15185550100".to_string()),
        location: None,
        trunk_name: Some("e911-trunk".to_string()),
        outcome: "placed".to_string(),
        placed_at: SystemTime::now(),
        notified: false,
    };
    let id = app.emergency.insert_emergency_call(&record).await.unwrap();
    assert!(id > 0);
    let mut b_record = record.clone();
    b_record.tenant_id = app.tenant_b;
    app.emergency
        .insert_emergency_call(&b_record)
        .await
        .unwrap();

    let calls = app
        .admin(Method::GET, "/api/v1/emergency/calls", None)
        .await;
    assert_eq!(calls.status, StatusCode::OK, "{}", calls.text);
    assert_eq!(calls.json.as_array().unwrap().len(), 1, "{}", calls.text);
    assert_eq!(calls.json[0]["dialed_number"], "911");
    assert_eq!(calls.json[0]["kind"], "live");
    assert_eq!(calls.json[0]["callback_number"], "+15185550100");
    assert_eq!(calls.json[0]["caller_username"], "ready");
    assert_eq!(calls.json[0]["notified"], false);
    app.emergency
        .mark_emergency_call_notified(id)
        .await
        .unwrap();
    let calls = app
        .admin(Method::GET, "/api/v1/emergency/calls", None)
        .await;
    assert_eq!(calls.json[0]["notified"], true, "{}", calls.text);

    app.finish().await;
}

// ─── Compliance basics ───────────────────────────────────────────────────

/// Poll the audit log until `pred` matches an entry (writes are async).
async fn wait_for_audit(app: &App, path: &str, pred: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..30 {
        let res = app.admin(Method::GET, path, None).await;
        if let Some(hit) = res
            .json
            .as_array()
            .and_then(|a| a.iter().find(|e| pred(e)).cloned())
        {
            return hit;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("audit entry never appeared at {path}");
}

#[tokio::test]
async fn compliance_policy_audit_log_export_and_retention() {
    use fcp_core::compliance::{AuditEvent, ComplianceStore};

    let app = require_app!();
    let days = |d: u64| Duration::from_secs(d * 86_400);

    // ── Policy ──
    let defaults = app
        .admin(Method::GET, "/api/v1/compliance/policy", None)
        .await;
    assert_eq!(defaults.status, StatusCode::OK, "{}", defaults.text);
    assert_eq!(defaults.json["configured"], false);
    assert!(defaults.json["cdr_retention_days"].is_null());
    assert_eq!(defaults.json["recording_consent"], "none");

    let bad_days = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "cdr_retention_days": 0 })),
        )
        .await;
    assert_eq!(
        bad_days.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_days.text
    );
    let announce_without_prompt = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "recording_consent": "announce" })),
        )
        .await;
    assert_eq!(
        announce_without_prompt.status,
        StatusCode::BAD_REQUEST,
        "{}",
        announce_without_prompt.text
    );
    let unknown_prompt = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "recording_consent": "announce", "recording_consent_prompt_id": "prompt-00000000-0000-0000-0000-000000000000" })),
        )
        .await;
    assert_eq!(
        unknown_prompt.status,
        StatusCode::BAD_REQUEST,
        "{}",
        unknown_prompt.text
    );

    let prompt = app
        .admin(
            Method::POST,
            "/api/v1/prompts",
            Some(json!({ "name": "consent", "category": "system", "source_type": "file" })),
        )
        .await;
    assert_eq!(prompt.status, StatusCode::CREATED, "{}", prompt.text);
    let prompt_id = prompt.json["id"].as_str().unwrap().to_string();

    let saved = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({
                "cdr_retention_days": 30,
                "sip_trace_retention_days": 30,
                "flow_trace_retention_days": 30,
                "audit_retention_days": 90,
                "recording_consent": "announce",
                "recording_consent_prompt_id": prompt_id
            })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json["configured"], true);
    assert_eq!(saved.json["cdr_retention_days"], 30);
    assert!(
        saved.json["recording_retention_days"].is_null(),
        "untouched stays forever"
    );
    assert_eq!(saved.json["recording_consent"], "announce");
    // Clearing a window with null, keeping the rest.
    let cleared = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "audit_retention_days": null })),
        )
        .await;
    assert!(
        cleared.json["audit_retention_days"].is_null(),
        "{}",
        cleared.text
    );
    assert_eq!(cleared.json["cdr_retention_days"], 30);
    let other = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/compliance/policy", None)
        .await;
    assert_eq!(
        other.json["configured"], false,
        "tenant B untouched: {}",
        other.text
    );

    // ── Audit log ──
    // The policy update above and a failed login must both be recorded.
    let entry = wait_for_audit(
        &app,
        "/api/v1/audit-log?action=PUT%20/api/v1/compliance/policy",
        |e| e["status"] == 200,
    )
    .await;
    assert_eq!(entry["actor"], "static_token", "{entry}");
    assert_eq!(entry["actor_kind"], "static_token");
    assert_eq!(entry["method"], "PUT");
    assert_eq!(entry["path"], "/api/v1/compliance/policy");
    assert_eq!(entry["tenant_id"], app.tenant_a.to_string());

    app.login("ghost", "nope").await;
    let failed_login = wait_for_audit(
        &app,
        "/api/v1/audit-log?action=POST%20/api/v1/auth/login",
        |e| e["status"] == 401,
    )
    .await;
    assert_eq!(failed_login["actor"], "anonymous", "{failed_login}");

    // A JWT session is recorded under its username, and tenant B's admin
    // sees none of tenant A's events.
    app.create_user_with_role("auditor", "tenant_admin").await;
    let jwt = app.login("auditor", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let created = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/devices",
            Some(
                json!({ "name": "audited", "device_type": "softphone", "tenant_id": app.tenant_a }),
            ),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let by_session = wait_for_audit(&app, "/api/v1/audit-log?actor=auditor", |e| {
        e["action"] == "POST /api/v1/devices" && e["status"] == 201
    })
    .await;
    assert_eq!(by_session["actor_kind"], "session", "{by_session}");
    // GET requests are not audited.
    let reads = app
        .admin(
            Method::GET,
            "/api/v1/audit-log?action=GET%20/api/v1/devices",
            None,
        )
        .await;
    assert!(reads.json.as_array().unwrap().is_empty(), "{}", reads.text);
    let b_admin = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/users",
            Some(json!({ "username": "b-auditor", "password": PASSWORD, "role": "tenant_admin" })),
        )
        .await;
    assert_eq!(b_admin.status, StatusCode::CREATED, "{}", b_admin.text);
    let jwt_b = app.login("b-auditor", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let b_view = app
        .send_as(&jwt_b, Method::GET, "/api/v1/audit-log?limit=200", None)
        .await;
    assert_eq!(b_view.status, StatusCode::OK, "{}", b_view.text);
    assert!(
        b_view
            .json
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["tenant_id"] == app.tenant_b.to_string()),
        "{}",
        b_view.text
    );

    // ── Export ──
    let (_, _) = app
        .seed_cdr_full(app.tenant_a, Some(5_000), Some(0.10))
        .await;
    let (_, _) = app.seed_cdr_full(app.tenant_a, None, None).await;
    let (_, _) = app.seed_cdr_full(app.tenant_b, Some(1_000), None).await;
    let export = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/cdrs/export", None)
        .await;
    assert_eq!(export.status, StatusCode::OK, "{}", export.text);
    assert!(export
        .header("content-type")
        .is_some_and(|ct| ct.starts_with("text/csv")));
    assert!(export
        .header("content-disposition")
        .is_some_and(|cd| cd.contains("cdrs.csv")));
    let lines: Vec<&str> = export.text.lines().collect();
    assert!(
        lines[0].starts_with("call_id,tenant_id,sip_call_id,start_time"),
        "{}",
        lines[0]
    );
    assert_eq!(lines.len(), 3, "header + 2 rows: {}", export.text);
    assert!(lines[1..]
        .iter()
        .all(|l| l.contains(&app.tenant_a.to_string())));
    let answered_only = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/cdrs/export?disposition=answered",
            None,
        )
        .await;
    assert_eq!(
        answered_only.text.lines().count(),
        2,
        "{}",
        answered_only.text
    );
    let all = app.admin(Method::GET, "/api/v1/cdrs/export", None).await;
    assert_eq!(all.text.lines().count(), 4, "platform-wide: {}", all.text);
    let exported = wait_for_audit(
        &app,
        "/api/v1/audit-log?action=GET%20/api/v1/cdrs/export",
        |e| e["status"] == 200,
    )
    .await;
    assert_eq!(exported["actor"], "static_token", "{exported}");

    // ── Retention ──
    let (old_call, old_sip) = app
        .seed_cdr_aged(app.tenant_a, Some(5_000), None, days(60))
        .await;
    let (new_call, _) = app.seed_cdr_full(app.tenant_a, Some(5_000), None).await;
    let (b_old, b_sip) = app
        .seed_cdr_aged(app.tenant_b, Some(5_000), None, days(60))
        .await;
    let old_ts = SystemTime::now() - days(60);
    app.seed_trace_at(&old_call, "ivr", FlowTraceEventType::Started, old_ts)
        .await;
    app.seed_trace(&new_call, "ivr", FlowTraceEventType::Started)
        .await;
    app.seed_sip_trace_at(
        &old_call.to_string(),
        &old_sip,
        app.tenant_a,
        "inbound",
        "INVITE",
        old_ts,
    )
    .await;
    app.seed_sip_trace(
        &new_call.to_string(),
        "new@10.0.0.5",
        app.tenant_a,
        "inbound",
        "INVITE",
    )
    .await;
    app.seed_sip_trace_at(
        &b_old.to_string(),
        &b_sip,
        app.tenant_b,
        "inbound",
        "INVITE",
        old_ts,
    )
    .await;
    // An old audit event for tenant A (its window was cleared, so it must survive).
    app.compliance
        .record_audit_event(&AuditEvent {
            id: 0,
            tenant_id: Some(app.tenant_a),
            actor: "static_token".into(),
            actor_kind: "static_token".into(),
            method: "DELETE".into(),
            path: "/api/v1/users/old".into(),
            status: 204,
            action: "DELETE /api/v1/users/:username".into(),
            detail: None,
            source_ip: None,
            occurred_at: SystemTime::now() - days(400),
        })
        .await
        .unwrap();

    let run = app
        .admin(Method::POST, "/api/v1/compliance/retention/run", None)
        .await;
    assert_eq!(run.status, StatusCode::OK, "{}", run.text);
    assert_eq!(run.json["cdrs_deleted"], 1, "{}", run.text);
    assert_eq!(run.json["sip_traces_deleted"], 1, "{}", run.text);
    assert_eq!(run.json["flow_traces_deleted"], 1, "{}", run.text);
    assert_eq!(
        run.json["audit_events_deleted"], 0,
        "window cleared: {}",
        run.text
    );
    assert_eq!(run.json["recordings_deleted"], 0);
    assert!(
        run.json["errors"].as_array().unwrap().is_empty(),
        "{}",
        run.text
    );

    assert_eq!(
        app.admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/cdrs/{old_call}"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND,
        "old CDR purged"
    );
    assert_eq!(
        app.admin_in(
            app.tenant_a,
            Method::GET,
            &format!("/api/v1/cdrs/{new_call}"),
            None
        )
        .await
        .status,
        StatusCode::OK,
        "recent CDR kept"
    );
    let b_kept = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/cdrs/{b_old}"),
            None,
        )
        .await;
    assert_eq!(
        b_kept.status,
        StatusCode::OK,
        "tenant B has no policy: {}",
        b_kept.text
    );
    let b_traces = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/calls/{b_old}/sip-messages"),
            None,
        )
        .await;
    assert_eq!(
        b_traces.json["messages"].as_array().unwrap().len(),
        1,
        "{}",
        b_traces.text
    );
    let old_audit = app
        .admin(
            Method::GET,
            "/api/v1/audit-log?action=DELETE%20/api/v1/users",
            None,
        )
        .await;
    assert_eq!(
        old_audit.json.as_array().unwrap().len(),
        1,
        "{}",
        old_audit.text
    );

    // Now set an audit window and run again: only the old audit row goes.
    app.admin(
        Method::PUT,
        "/api/v1/compliance/policy",
        Some(json!({ "audit_retention_days": 90 })),
    )
    .await;
    let second = app
        .admin(Method::POST, "/api/v1/compliance/retention/run", None)
        .await;
    assert_eq!(second.json["audit_events_deleted"], 1, "{}", second.text);
    assert_eq!(second.json["cdrs_deleted"], 0, "{}", second.text);

    let runs = app
        .admin(Method::GET, "/api/v1/compliance/retention/runs", None)
        .await;
    assert_eq!(runs.status, StatusCode::OK, "{}", runs.text);
    assert_eq!(runs.json.as_array().unwrap().len(), 2, "{}", runs.text);
    assert_eq!(
        runs.json[1]["cdrs_deleted"], 1,
        "oldest last: {}",
        runs.text
    );

    app.finish().await;
}

// ─── Live conference controls (proxied to the conference server) ─────────

/// One request the mock conference server received.
#[derive(Debug, Clone)]
struct Upstream {
    method: String,
    path: String,
    tenant: Option<String>,
    bearer: Option<String>,
    body: Value,
}

/// A stand-in conference server on a loopback port: records every request
/// and answers by path the way the real one does (room 9999 is "not
/// active anywhere", everything else succeeds with an echo).
async fn mock_conference_server() -> (String, Arc<std::sync::Mutex<Vec<Upstream>>>) {
    use axum::extract::State;
    use axum::routing::any;
    let seen: Arc<std::sync::Mutex<Vec<Upstream>>> = Arc::default();
    async fn record(
        State(seen): State<Arc<std::sync::Mutex<Vec<Upstream>>>>,
        method: Method,
        uri: axum::http::Uri,
        headers: HeaderMap,
        bytes: Bytes,
    ) -> (StatusCode, axum::Json<Value>) {
        let method = method.to_string();
        let path = uri.path().to_string();
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let tenant = header("x-fcp-tenant-id");
        let bearer = header(header::AUTHORIZATION.as_str())
            .and_then(|v| v.strip_prefix("Bearer ").map(str::to_string));
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        seen.lock().unwrap().push(Upstream {
            method: method.clone(),
            path: path.clone(),
            tenant,
            bearer,
            body: body.clone(),
        });
        if path.starts_with("/conferences/9999/") {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": "Conference room not active" })),
            );
        }
        (
            StatusCode::OK,
            axum::Json(json!({ "status": "ok", "method": method, "path": path, "body": body })),
        )
    }
    let router = Router::new()
        .fallback(any(record))
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://127.0.0.1:{port}"), seen)
}

// ─── Conference recordings ───────────────────────────────────────────────

/// Put a recording in the registry directly, as a conference node does.
async fn register_recording(
    app: &App,
    tenant: TenantId,
    room: &str,
    node: &str,
    finished: bool,
) -> fcp_core::Recording {
    let store =
        fcp_storage_postgres::recording::PostgresRecordingStore::new(app.storage.pool().clone());
    let mut r = fcp_core::Recording::starting(
        tenant,
        room,
        node,
        format!("/var/fcp/recordings/{room}.webm"),
        fcp_core::RecordingFormat::Webm,
        fcp_core::RecordingTrigger::Api,
    );
    r.participants = 2;
    fcp_core::recording::RecordingStore::create_recording(&store, &r)
        .await
        .unwrap();
    if finished {
        let ended = SystemTime::now();
        fcp_core::recording::RecordingStore::finish_recording(&store, &r.id, ended, 12, 3_400)
            .await
            .unwrap();
        r.ended_at = Some(ended);
        r.duration_secs = Some(12);
        r.size_bytes = Some(3_400);
    }
    r
}

#[tokio::test]
async fn recordings_are_listed_scoped_and_deleted_through_their_node() {
    let (url, seen) = mock_conference_server().await;
    let Some(app) = App::with_conference_server(&url).await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
        return;
    };
    let done = register_recording(&app, app.tenant_a, "8001", "conf-a", true).await;
    let running = register_recording(&app, app.tenant_a, "8002", "conf-b", false).await;
    let other = register_recording(&app, app.tenant_b, "9001", "conf-a", true).await;

    // The tenant's own, newest first, with the row's detail.
    let list = app.admin(Method::GET, "/api/v1/recordings", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let rows = list.json["recordings"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{}", list.text);
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&done.id.to_string().as_str()));
    assert!(
        !ids.contains(&other.id.to_string().as_str()),
        "other tenant"
    );
    let first = rows
        .iter()
        .find(|r| r["id"] == done.id.to_string())
        .unwrap();
    assert_eq!(first["room_number"], "8001");
    assert_eq!(first["format"], "webm");
    assert_eq!(first["node_id"], "conf-a");
    assert_eq!(first["file_name"], "8001.webm");
    assert_eq!(first["duration_secs"], 12);
    assert_eq!(first["size_bytes"], 3400);
    assert_eq!(first["running"], false);
    assert_eq!(
        first["download_url"],
        format!("/v1/recordings/{}/download", done.id)
    );

    // Filters.
    let only_running = app
        .admin(Method::GET, "/api/v1/recordings?running=true", None)
        .await;
    let rows = only_running.json["recordings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], running.id.to_string());
    let by_room = app
        .admin(Method::GET, "/api/v1/recordings?room_number=8001", None)
        .await;
    assert_eq!(by_room.json["recordings"].as_array().unwrap().len(), 1);

    // One recording, and another tenant's is simply not found.
    let got = app
        .admin(
            Method::GET,
            &format!("/api/v1/recordings/{}", done.id),
            None,
        )
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["room_number"], "8001");
    let hidden = app
        .admin(
            Method::GET,
            &format!("/api/v1/recordings/{}", other.id),
            None,
        )
        .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.text);

    // A recording still being written cannot be deleted.
    let busy = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/recordings/{}", running.id),
            None,
        )
        .await;
    assert_eq!(busy.status, StatusCode::CONFLICT, "{}", busy.text);

    // Deleting takes the file through the node holding it, then the row.
    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/recordings/{}", done.id),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::OK, "{}", gone.text);
    assert_eq!(gone.json["file_deleted"], true, "{}", gone.text);
    {
        let seen = seen.lock().unwrap();
        let up = seen.last().expect("the conference server was called");
        assert_eq!(up.method, "DELETE");
        assert_eq!(up.path, "/recordings/8001.webm");
        assert_eq!(
            up.tenant.as_deref(),
            Some(app.tenant_a.to_string().as_str())
        );
        assert_eq!(up.bearer.as_deref(), Some(CONFERENCE_TOKEN));
    }
    let after = app
        .admin(
            Method::GET,
            &format!("/api/v1/recordings/{}", done.id),
            None,
        )
        .await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);

    app.finish().await;
}

#[tokio::test]
async fn the_retention_sweep_takes_only_conference_recordings_a_node_abandoned() {
    let app = require_app!();
    let days = |d: u64| Duration::from_secs(d * 86_400);
    let store =
        fcp_storage_postgres::recording::PostgresRecordingStore::new(app.storage.pool().clone());
    let aged = |room: &str, age: Duration| {
        let mut r = fcp_core::Recording::starting(
            app.tenant_a,
            room,
            "conf-a",
            format!("/var/fcp/recordings/{room}.webm"),
            fcp_core::RecordingFormat::Webm,
            fcp_core::RecordingTrigger::Auto,
        );
        r.started_at = SystemTime::now() - age;
        r
    };
    // Both are past a 30-day window; only the first is past the week of
    // grace the holding node gets to delete file and row itself.
    let abandoned = aged("8001", days(400));
    let recent = aged("8002", days(32));
    let running = aged("8003", days(400));
    for r in [&abandoned, &recent, &running] {
        fcp_core::recording::RecordingStore::create_recording(&store, r)
            .await
            .unwrap();
    }
    for r in [&abandoned, &recent] {
        fcp_core::recording::RecordingStore::finish_recording(
            &store,
            &r.id,
            SystemTime::now() - days(1),
            10,
            100,
        )
        .await
        .unwrap();
    }

    let set = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "recording_retention_days": 30 })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);

    let run = app
        .admin(Method::POST, "/api/v1/compliance/retention/run", None)
        .await;
    assert_eq!(run.status, StatusCode::OK, "{}", run.text);
    assert_eq!(run.json["conference_recordings_deleted"], 1, "{}", run.text);

    let left = app.admin(Method::GET, "/api/v1/recordings", None).await;
    let rooms: Vec<&str> = left.json["recordings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["room_number"].as_str().unwrap())
        .collect();
    assert_eq!(rooms.len(), 2, "{}", left.text);
    assert!(rooms.contains(&"8002"), "still inside the grace period");
    assert!(rooms.contains(&"8003"), "a running recording never expires");

    // The run is on the record with its own count.
    let runs = app
        .admin(Method::GET, "/api/v1/compliance/retention/runs", None)
        .await;
    assert_eq!(runs.status, StatusCode::OK, "{}", runs.text);
    assert_eq!(
        runs.json[0]["conference_recordings_deleted"], 1,
        "{}",
        runs.text
    );

    app.finish().await;
}

/// Whether any string or number in the JSON equals `needle`. Used for
/// "this secret is not in the response", where a substring search over
/// the serialized body gives false positives against uuids and
/// timestamps.
fn json_has_value(v: &Value, needle: &str) -> bool {
    match v {
        Value::String(s) => s == needle,
        Value::Number(n) => n.to_string() == needle,
        Value::Array(a) => a.iter().any(|x| json_has_value(x, needle)),
        Value::Object(o) => o.values().any(|x| json_has_value(x, needle)),
        _ => false,
    }
}

async fn create_meeting(app: &App, room: &str, owner: Option<&str>) {
    let res = app
        .admin(
            Method::POST,
            "/api/v1/conferences/meetings",
            Some(json!({
                "room_number": room,
                "name": format!("Room {room}"),
                "owner_id": owner,
            })),
        )
        .await;
    assert_eq!(
        res.status,
        StatusCode::CREATED,
        "create meeting: {}",
        res.text
    );
}

#[tokio::test]
async fn live_conference_controls_are_proxied_with_the_meetings_tenant() {
    let (url, seen) = mock_conference_server().await;
    let Some(app) = App::with_conference_server(&url).await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
        return;
    };
    create_meeting(&app, "8000", None).await;
    create_meeting(&app, "9999", None).await;

    // Status: forwarded with the meeting's tenant and the server's token;
    // the upstream body comes back as it is.
    let res = app
        .admin(Method::GET, "/api/v1/conferences/live/8000/video", None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.json["path"], "/conferences/8000/video");
    {
        let seen = seen.lock().unwrap();
        let up = seen.last().expect("upstream was called");
        assert_eq!(up.method, "GET");
        assert_eq!(
            up.tenant.as_deref(),
            Some(app.tenant_a.to_string().as_str())
        );
        assert_eq!(up.bearer.as_deref(), Some(CONFERENCE_TOKEN));
    }

    // Layout: the body travels; pin / spotlight / participant video take
    // their verbs and paths through unchanged.
    let res = app
        .admin(
            Method::PUT,
            "/api/v1/conferences/live/8000/video/layout",
            Some(json!({ "layout": "pip" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.json["body"], json!({ "layout": "pip" }));
    for (method, path) in [
        (Method::POST, "/video/pin/p-1"),
        (Method::DELETE, "/video/pin"),
        (Method::POST, "/video/spotlight/p-1"),
        (Method::DELETE, "/video/spotlight"),
        (Method::POST, "/participants/p-1/video/off"),
        (Method::POST, "/participants/p-1/video/on"),
    ] {
        let res = app
            .admin(
                method.clone(),
                &format!("/api/v1/conferences/live/8000{path}"),
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{method} {path}: {}", res.text);
        let seen = seen.lock().unwrap();
        let up = seen.last().unwrap();
        assert_eq!(up.method, method.as_str());
        assert_eq!(up.path, format!("/conferences/8000{path}"));
    }
    let res = app
        .admin(
            Method::POST,
            "/api/v1/conferences/live/8000/dial-out",
            Some(json!({ "destination": "1001", "greeting_tts": "hi" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.json["body"]["destination"], "1001");

    // Decided here, never forwarded: a bad participant video state, a
    // room with no meeting, a meeting in another tenant.
    let before = seen.lock().unwrap().len();
    let res = app
        .admin(
            Method::POST,
            "/api/v1/conferences/live/8000/participants/p-1/video/sideways",
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
    let res = app
        .admin(Method::GET, "/api/v1/conferences/live/8001/video", None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.text);
    let res = app
        .send(
            Method::GET,
            "/api/v1/conferences/live/8000/video",
            &[
                (AUTH_HEADER, BEARER),
                ("x-fcp-tenant-id", &app.tenant_b.to_string()),
            ],
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.text);
    assert_eq!(
        seen.lock().unwrap().len(),
        before,
        "nothing reached the upstream"
    );

    // The upstream's own 404 (room not active on any node) passes through.
    let res = app
        .admin(Method::GET, "/api/v1/conferences/live/9999/video", None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.text);
    assert_eq!(res.json["error"], "Conference room not active");

    // End users are kept out of the admin routes.
    app.create_user_with_role("viewer", "end_user").await;
    let jwt = app.login("viewer", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send_as(
            &jwt,
            Method::GET,
            "/api/v1/conferences/live/8000/video",
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.text);
}

#[tokio::test]
async fn live_conference_controls_need_a_conference_server() {
    let app = require_app!();
    create_meeting(&app, "8000", None).await;
    let res = app
        .admin(Method::GET, "/api/v1/conferences/live/8000/video", None)
        .await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE, "{}", res.text);
}

#[tokio::test]
async fn owners_dial_out_of_their_own_conferences() {
    let (url, seen) = mock_conference_server().await;
    let Some(app) = App::with_conference_server(&url).await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
        return;
    };
    app.create_user_with_role("host", "end_user").await;
    app.create_user_with_role("guest", "end_user").await;
    create_meeting(&app, "8000", Some("host")).await;
    let meetings = app
        .admin(Method::GET, "/api/v1/conferences/meetings", None)
        .await;
    let id = meetings.json.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let host = app.login("host", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let res = app
        .send_as(
            &host,
            Method::POST,
            &format!("/api/v1/me/conferences/{id}/dial-out"),
            Some(json!({ "destination": "+15550001111" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    {
        let seen = seen.lock().unwrap();
        let up = seen.last().expect("upstream was called");
        assert_eq!(up.path, "/conferences/8000/dial-out");
        assert_eq!(up.body["destination"], "+15550001111");
        assert_eq!(
            up.tenant.as_deref(),
            Some(app.tenant_a.to_string().as_str())
        );
    }

    // Somebody else's conference does not exist as far as this user knows.
    let guest = app.login("guest", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let before = seen.lock().unwrap().len();
    let res = app
        .send_as(
            &guest,
            Method::POST,
            &format!("/api/v1/me/conferences/{id}/dial-out"),
            Some(json!({ "destination": "+15550001111" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.text);
    assert_eq!(seen.lock().unwrap().len(), before);
}
