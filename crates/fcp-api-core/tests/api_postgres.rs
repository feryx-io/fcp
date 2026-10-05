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
use fcp_core::storage::{Registration, RegistrationStore, WebhookStore};
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
    /// The storage the API runs on, for a test to run a worker against.
    dyn_storage: Arc<DynStorage>,
    /// The API's state, for a test to run a sweep against.
    state: ApiState,
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

/// The API's configuration in these tests.
fn test_api_config() -> ApiConfig {
    ApiConfig {
        auth_enabled: true,
        auth_token: Some(ADMIN_TOKEN.to_string()),
        provisioning_require_https: true,
        // A stand-in identity provider listens on loopback.
        sso: fcp_core::ApiSsoConfig {
            public_url: Some(SSO_PUBLIC_URL.to_string()),
            allow_private_issuers: true,
        },
        ..ApiConfig::default()
    }
}

impl App {
    /// The app with vendors' redirection services where `config` says:
    /// the stand-ins (`docs/PROVISIONING.md` §10).
    fn with_redirection(mut self, config: fcp_core::ApiRedirectionConfig) -> Self {
        let api_config = ApiConfig {
            redirection: config,
            ..test_api_config()
        };
        self.router = create_router(self.state.clone(), &api_config);
        self
    }

    /// The app keeping firmware images in `dir` (`docs/PROVISIONING.md` §11).
    fn with_firmware(mut self, dir: &std::path::Path) -> Self {
        let api_config = ApiConfig {
            firmware: fcp_core::ApiFirmwareConfig {
                directory: Some(dir.to_string_lossy().into_owned()),
                max_image_mb: 1,
                s3: None,
            },
            ..test_api_config()
        };
        self.router = create_router(self.state.clone(), &api_config);
        self
    }

    /// An app whose live conference endpoints proxy to `conference_url`.
    async fn with_conference_server(conference_url: &str) -> Option<Self> {
        Self::build(DeploymentMode::default(), Some(conference_url.to_string())).await
    }

    async fn build(mode: DeploymentMode, conference_url: Option<String>) -> Option<Self> {
        Self::build_with(mode, conference_url, false, None).await
    }

    /// An app whose webhooks may name loopback and private addresses, so a
    /// test's own receiver can be one (`[webhooks] allow_private_targets`).
    async fn with_private_webhooks() -> Option<Self> {
        Self::build_with(DeploymentMode::default(), None, true, None).await
    }

    /// An app that is a site of a cloud (`[api] managed_by`): synced
    /// objects are the cloud's, and its users come from there.
    async fn as_site() -> Option<Self> {
        Self::build_with(
            DeploymentMode::default(),
            None,
            false,
            Some("https://cloud.test".to_string()),
        )
        .await
    }

    async fn build_with(
        mode: DeploymentMode,
        conference_url: Option<String>,
        private_webhooks: bool,
        managed_by: Option<String>,
    ) -> Option<Self> {
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
        let dyn_storage =
            Arc::new(
                DynStorage::from_all_in_one(Arc::clone(&storage))
                    .with_api_token_store(api_tokens)
                    .with_totp_store(Arc::clone(&storage) as Arc<dyn fcp_core::totp::TotpStore>)
                    .with_identity_store(
                        Arc::clone(&storage) as Arc<dyn fcp_core::identity::IdentityStore>
                    )
                    .with_login_session_store(Arc::new(
                        fcp_storage_postgres::login_session::PostgresLoginSessionStore::new(
                            storage.pool().clone(),
                        ),
                    )
                        as Arc<dyn fcp_core::session::LoginSessionStore>)
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
                    .with_recording_store(Arc::new(PostgresRecordingStore::new(
                        storage.pool().clone(),
                    ))
                        as Arc<dyn fcp_core::recording::RecordingStore>)
                    .with_call_recording_store(Arc::new(PostgresRecordingStore::new(
                        storage.pool().clone(),
                    ))
                        as Arc<dyn fcp_core::recording::CallRecordingStore>)
                    .with_subject_data_store(Arc::clone(&storage)
                        as Arc<dyn fcp_core::subject_request::SubjectDataStore>)
                    .with_subject_erase_store(Arc::clone(&storage)
                        as Arc<dyn fcp_core::subject_erasure::SubjectEraseStore>)
                    .with_offboarding_store(
                        Arc::clone(&storage) as Arc<dyn fcp_core::offboarding::OffboardingStore>
                    )
                    .with_group_store(Arc::clone(&storage) as Arc<dyn fcp_core::group::GroupStore>)
                    .with_provisioning_store(
                        Arc::clone(&storage) as Arc<dyn fcp_core::provisioning::ProvisioningStore>
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
            Some(Arc::clone(&dyn_storage)),
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
        .with_managed_by(managed_by)
        .with_conference_server_url(conference_url)
        .with_conference_server_auth_token(Some(CONFERENCE_TOKEN.to_string()))
        .with_profile_pictures_dir(dir.path().join("avatars"))
        .with_voicemail_greetings_dir(dir.path().join("greetings"))
        .with_prompts_dir(dir.path().join("prompts"))
        .with_webhook_dispatcher(Some(Arc::new(fcp_webhooks::WebhookDispatcher::new(
            Arc::clone(dyn_storage.webhook_store()),
            fcp_webhooks::WebhookDispatcherConfig {
                request_timeout: Duration::from_secs(5),
                allow_private_targets: private_webhooks,
                ..Default::default()
            },
        ))));

        let api_config = test_api_config();

        Some(Self {
            router: create_router(state.clone(), &api_config),
            state,
            storage,
            traces,
            sip_traces,
            emergency,
            compliance,
            dyn_storage,
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
        let verb = method.clone();
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
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() && path.starts_with("/api/v1/") {
            check_shape(path, "", &body);
            // docs/DEVELOPER_SURFACE.md §3, phase 2c: no list is a bare array.
            assert!(
                !body.is_array(),
                "{path} answered a bare array; every list is {{data, next_cursor}}"
            );
        }
        if path.starts_with("/api/v1/") {
            check_contract(&verb, path, status, &headers, &body);
        }
        // A list's items, for the tests that read them; `body` keeps the
        // envelope for those that page.
        let json = match &body {
            Value::Object(map) if map.contains_key("data") && map.contains_key("next_cursor") => {
                body["data"].clone()
            }
            _ => body.clone(),
        };
        Response {
            body,
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
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Response {
            body: json.clone(),
            status,
            headers,
            json,
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
        cost: Option<fcp_core::billing::Decimal>,
    ) -> CallId {
        self.seed_cdr_full(tenant, answered_ms, cost).await.0
    }

    /// Like [`seed_cdr`] but also returns the SIP Call-ID it recorded.
    async fn seed_cdr_full(
        &self,
        tenant: TenantId,
        answered_ms: Option<u64>,
        cost: Option<fcp_core::billing::Decimal>,
    ) -> (CallId, String) {
        self.seed_cdr_aged(tenant, answered_ms, cost, Duration::from_secs(300))
            .await
    }

    /// Like [`seed_cdr_full`] with the call starting `age` ago.
    async fn seed_cdr_aged(
        &self,
        tenant: TenantId,
        answered_ms: Option<u64>,
        cost: Option<fcp_core::billing::Decimal>,
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
            recording_consent: None,
            cost,
            currency: cost.map(|_| "USD".to_string()),
            direction: None,
            node_id: None,
            site_id: None,
            region: None,
            correlation_id: None,
            trunk_name: None,
            trunk_id: None,
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
                source_address: None,
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
                tenant_id: None,
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

    /// The tenant's provisioning key, made on first use.
    async fn provisioning_key(&self) -> String {
        let shown = self
            .admin(Method::GET, "/api/v1/provisioning/key", None)
            .await;
        let key = if shown.status == StatusCode::OK {
            shown
        } else {
            self.admin(Method::POST, "/api/v1/provisioning/key", None)
                .await
        };
        key.json["key"]
            .as_str()
            .expect("provisioning key")
            .to_string()
    }

    /// Provisioning request as a phone would send it: HTTPS via the proxy
    /// header, the tenant's URL, and its device credentials (Basic, the
    /// device's provisioning token) when it has them.
    async fn provision(&self, mac: &str, device_token: Option<&str>) -> Response {
        use base64::Engine;
        let key = self.provisioning_key().await;
        let basic = device_token.map(|t| {
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("phone:{t}"))
            )
        });
        let mut headers = vec![("x-forwarded-proto", "https")];
        if let Some(basic) = basic.as_deref() {
            headers.push(("authorization", basic));
        }
        self.send(
            Method::GET,
            &format!("/provision/{key}/{mac}"),
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

/// docs/DEVELOPER_SURFACE.md §3, phase 2b: every response any test sees is
/// checked — a time is an RFC 3339 string, never a number or serde's
/// `{secs_since_epoch, …}`; an id is never a bare UUID where its type has a
/// prefix.
fn check_shape(path: &str, at: &str, value: &Value) {
    match value {
        Value::Object(map) => {
            assert!(
                !map.contains_key("secs_since_epoch"),
                "{path}: {at} is serde's SystemTime shape: {value}"
            );
            for (key, v) in map {
                let here = format!("{at}.{key}");
                if is_time_key(key) {
                    match v {
                        Value::Null => {}
                        Value::String(s) => assert!(
                            chrono::DateTime::parse_from_rfc3339(s).is_ok(),
                            "{path}: {here} = {s:?} is not RFC 3339"
                        ),
                        other => panic!("{path}: {here} = {other} is not an RFC 3339 string"),
                    }
                    continue;
                }
                if is_prefixed_id_key(key) && !unprefixed_resource(path, key) {
                    if let Value::String(s) = v {
                        assert!(
                            uuid::Uuid::parse_str(s).is_err(),
                            "{path}: {here} = {s:?} is a bare UUID; ids carry their prefix"
                        );
                    }
                }
                check_shape(path, &here, v);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                check_shape(path, &format!("{at}[{i}]"), v);
            }
        }
        _ => {}
    }
}

/// Seconds since the epoch of an RFC 3339 time in a response.
fn rfc3339_secs(value: &Value) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value.as_str().expect("an RFC 3339 string"))
        .expect("RFC 3339")
        .timestamp()
}

/// The OpenAPI contract as JSON, made once.
fn spec() -> &'static Value {
    static SPEC: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SPEC.get_or_init(|| {
        use utoipa::OpenApi;
        serde_json::to_value(fcp_api_core::openapi::ApiDoc::openapi()).expect("the spec")
    })
}

/// The spec's path template that `path` falls under: the one matching
/// segment by segment with the most literal segments.
fn template_for(path: &str) -> Option<String> {
    let path = path.split('?').next().unwrap();
    let parts: Vec<&str> = path.split('/').collect();
    let mut best: Option<(usize, String)> = None;
    for template in spec()["paths"].as_object().unwrap().keys() {
        let t: Vec<&str> = template.split('/').collect();
        if t.len() != parts.len() {
            continue;
        }
        let mut literal = 0;
        let fits = t.iter().zip(&parts).all(|(t, p)| {
            if t.starts_with('{') {
                true
            } else if t == p {
                literal += 1;
                true
            } else {
                false
            }
        });
        if fits && best.as_ref().is_none_or(|(n, _)| literal > *n) {
            best = Some((literal, template.clone()));
        }
    }
    best.map(|(_, t)| t)
}

/// docs/DEVELOPER_SURFACE.md §3, phase 3: every response a test sees is
/// what the contract says — the route is in the spec, the status is one it
/// documents, and a JSON body validates against that status's schema.
fn check_contract(
    method: &Method,
    path: &str,
    status: StatusCode,
    headers: &HeaderMap,
    body: &Value,
) {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static VALIDATORS: OnceLock<Mutex<HashMap<String, Arc<jsonschema::Validator>>>> =
        OnceLock::new();

    // A CORS preflight is the CORS layer's, not an operation's.
    if *method == Method::OPTIONS {
        return;
    }
    // The router's own refusal of a path or a method nothing serves is not
    // an operation's answer.
    let refused_by_router = matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    );
    let Some(template) = template_for(path) else {
        assert!(
            refused_by_router,
            "{method} {path}: no operation in the contract"
        );
        return;
    };
    let op = &spec()["paths"][&template][method.as_str().to_lowercase()];
    if !op.is_object() {
        assert!(
            refused_by_router,
            "{method} {path}: no {method} on {template} in the contract"
        );
        return;
    }
    let code = status.as_u16().to_string();
    let response = if op["responses"][&code].is_object() {
        &op["responses"][&code]
    } else if !status.is_success() && op["responses"]["default"].is_object() {
        &op["responses"]["default"]
    } else {
        panic!("{method} {path}: status {code} is not in the contract for {template}");
    };
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    let schema = &response["content"]["application/json"]["schema"];
    if !json || schema.is_null() {
        return;
    }
    let key = format!("{method} {template} {code}");
    let validator = {
        let mut cache = VALIDATORS.get_or_init(Default::default).lock().unwrap();
        cache
            .entry(key.clone())
            .or_insert_with(|| {
                let root = serde_json::json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "allOf": [schema],
                    "components": spec()["components"],
                });
                Arc::new(jsonschema::validator_for(&root).expect("the contract's schema compiles"))
            })
            .clone()
    };
    let errors: Vec<String> = validator
        .iter_errors(body)
        .take(5)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{key}: the body is not what the contract says:\n  {}\nbody: {body}",
        errors.join("\n  ")
    );
}

/// A key naming a moment (not a duration: `_ms`, `_secs`, `_seconds` and
/// `_days` stay numbers).
fn is_time_key(key: &str) -> bool {
    key.ends_with("_at")
        || key.ends_with("_time")
        || key.ends_with("_until")
        || key.ends_with("_since")
        || matches!(key, "timestamp" | "expires" | "last_seen" | "when")
}

/// Resources whose ids no `fcp_core::identifiers` type prefixes: an audit
/// entry, a billing transaction, a rate plan, a voicemail mailbox, a
/// webhook delivery. Their ids are plain UUIDs (`docs/API_TOKENS.md` §6).
fn unprefixed_resource(path: &str, key: &str) -> bool {
    (key == "id" && path.starts_with("/api/v1/webhooks/") && path.contains("/deliveries"))
        || (key == "id"
            && [
                "/api/v1/audit-log",
                "/api/v1/billing/balance/transactions",
                "/api/v1/rate-plans",
                "/api/v1/voicemail/mailboxes",
                "/api/v1/me/voicemail",
            ]
            .iter()
            .any(|p| path.starts_with(p)))
        || key == "mailbox_id"
}

/// A key naming one of the identifiers `fcp_core::identifiers` gives a
/// prefix.
fn is_prefixed_id_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "tenant_id"
            | "user_id"
            | "device_id"
            | "location_id"
            | "number_id"
            | "phone_number_id"
            | "binding_id"
            | "line_binding_id"
            | "extension_id"
            | "mailbox_id"
            | "site_id"
            | "schedule_id"
            | "class_of_service_id"
            | "hold_id"
            | "prompt_id"
            | "template_id"
            | "token_id"
            | "network_id"
            | "meeting_id"
            | "webhook_id"
            | "session_id"
            | "request_id"
    )
}

/// Where the test admin says browsers reach it (`[api.sso] public_url`).
const SSO_PUBLIC_URL: &str = "https://pbx.test";
const PASSWORD: &str = "Str0ng-Passw0rd!";
const BEARER: &str = "Bearer test-admin-token";

struct Response {
    /// The whole body; `json` is a list's `data` when the body is the list
    /// envelope.
    body: Value,
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
    // 32 characters: what a vendor's redirection service hands a phone.
    assert!(
        token.starts_with("fcp_") && token.len() == 32,
        "generated token: {token}"
    );

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

/// A shared line (`docs/ENTERPRISE_CALLING.md` §3.9): set on a user's
/// line through the API, refused on a number that is not a user's, and
/// rendered as the second account of every device bound to it.
#[tokio::test]
async fn a_shared_line_is_set_through_the_api_and_provisioned_as_a_second_account() {
    let app = require_app!();
    let owner = app.create_user("dana", Some("Sip-Secret-456")).await;
    let (device, token) = app
        .create_phone("Dana's phone", "AA:BB:CC:DD:EE:31", Some(&owner))
        .await;
    let template = app
        .create_template(
            app.tenant_a,
            "account.2.enable=${shared_line_enabled}\n\
             account.2.user_name=${shared_line_number}\n\
             keys=${shared_line_appearances}\n\
             cisco=${shared_line_yes_no}\n",
        )
        .await;
    let attached = app
        .patch_device(&device, json!({ "config_template_id": template }))
        .await;
    assert_eq!(attached.status, StatusCode::OK, "{}", attached.text);

    // On no shared line, the second account is off and nothing is left
    // unrendered.
    let off = app.provision("aabbccddee31", Some(&token)).await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.text);
    assert!(off.text.contains("account.2.enable=0"), "{}", off.text);
    assert!(
        off.text.contains("keys=0") && off.text.contains("cisco=No"),
        "{}",
        off.text
    );
    assert!(!off.text.contains("${"), "{}", off.text);

    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550003333", "tenant_id": app.tenant_a })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
    let refused = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({
                "phone_number_id": number.json["id"],
                "binding_type": "voicemail",
                "tenant_id": app.tenant_a,
                "shared": true,
            })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    let binding = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({
                "phone_number_id": number.json["id"],
                "owner_user_id": owner,
                "binding_type": "user",
                "tenant_id": app.tenant_a,
                "devices": [{
                    "device_id": device,
                    "priority": 1,
                    "line_appearance": 2,
                    "shared_line": true,
                    "enabled": true,
                }],
                "shared": true,
                "max_appearances": 3,
                "barge_in": true,
            })),
        )
        .await;
    assert_eq!(binding.status, StatusCode::CREATED, "{}", binding.text);
    assert_eq!(binding.json["shared"], true);
    assert_eq!(binding.json["barge_in"], true);
    assert_eq!(binding.json["max_appearances"], 3);
    let binding_id = binding.json["id"].as_str().expect("binding id").to_string();

    let too_many = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/line-bindings/{binding_id}"),
            Some(json!({ "max_appearances": 30 })),
        )
        .await;
    assert_eq!(
        too_many.status,
        StatusCode::BAD_REQUEST,
        "{}",
        too_many.text
    );

    let on = app.provision("aabbccddee31", Some(&token)).await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.text);
    assert!(on.text.contains("account.2.enable=1"), "{}", on.text);
    assert!(
        on.text.contains("account.2.user_name=+15550003333"),
        "{}",
        on.text
    );
    assert!(
        on.text.contains("keys=3") && on.text.contains("cisco=Yes"),
        "{}",
        on.text
    );

    let unshared = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/line-bindings/{binding_id}"),
            Some(json!({ "shared": false })),
        )
        .await;
    assert_eq!(unshared.status, StatusCode::OK, "{}", unshared.text);
    assert_eq!(unshared.json["shared"], false);
    let off_again = app.provision("aabbccddee31", Some(&token)).await;
    assert!(
        off_again.text.contains("account.2.enable=0"),
        "{}",
        off_again.text
    );

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

    let key = app.provisioning_key().await;
    failures.push((
        "plain http",
        app.send(
            Method::GET,
            &format!("/provision/{key}/aabbccddee05"),
            &[],
            None,
        )
        .await,
    ));
    failures.push((
        "another key",
        app.send(
            Method::GET,
            &format!("/provision/{}/aabbccddee05", "0".repeat(32)),
            &[("x-forwarded-proto", "https")],
            None,
        )
        .await,
    ));
    // The old route, with the deployment's token, is gone.
    let old = app
        .send(
            Method::GET,
            "/api/v1/provisioning/aabbccddee05",
            &[
                ("x-forwarded-proto", "https"),
                ("x-fcp-provisioning-token", "test-provisioning-shared-token"),
                ("x-fcp-device-provisioning-token", &token),
            ],
            None,
        )
        .await;
    assert_eq!(old.status, StatusCode::NOT_FOUND, "{}", old.text);

    // Device-level checks.
    failures.push((
        "unknown MAC",
        app.provision("001122334455", Some(&token)).await,
    ));
    // Credentials a phone sends are checked: wrong ones are asked again.
    let wrong = app
        .provision("aabbccddee05", Some("fcp_prov_not_the_right_token_at_all"))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.text);

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
    let cookie = refresh_cookie(&ok).expect("the refresh cookie");

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

    // Refresh, with the session's cookie, issues a new usable token.
    let refreshed = app
        .with_cookie(Method::POST, "/api/v1/auth/refresh", &cookie)
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

    // Scoped token: reaches what it names and nothing else.
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

    let reached = app
        .send_as(&secret, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(reached.status, StatusCode::OK, "{}", reached.text);
    // 403 (not 401) proves the token authenticated but lacks the permission.
    for (method, path) in [
        (Method::GET, "/api/v1/users"),
        (Method::POST, "/api/v1/devices"),
        (Method::GET, "/api/v1/api-tokens"),
    ] {
        let denied = app.send_as(&secret, method.clone(), path, None).await;
        assert_eq!(
            denied.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            denied.text
        );
    }

    // `admin` from the static token: everything a tenant admin holds,
    // scoped to its tenant.
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

/// Log a user in; returns the session JWT.
async fn session(app: &App, username: &str) -> String {
    let res = app.login(username, PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "login {username}: {}", res.text);
    res.json["token"].as_str().expect("jwt").to_string()
}

/// Make an admin token as `jwt`; returns (id, secret).
async fn make_token(app: &App, jwt: &str, permissions: Value) -> (String, String) {
    let res = app
        .send_as(
            jwt,
            Method::POST,
            "/api/v1/api-tokens",
            Some(json!({ "name": "t", "permissions": permissions })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
    (
        res.json["token"]["id"].as_str().unwrap().to_string(),
        res.json["secret"].as_str().unwrap().to_string(),
    )
}

/// docs/DEVELOPER_SURFACE.md §3, phase 1: a token never does more than its
/// maker, a scoped token reaches what it names, revoking keeps the row,
/// rotating honours the old secret for its grace, and a personal token
/// reaches only its maker's `/me` routes.
#[tokio::test]
async fn api_tokens_never_exceed_their_maker() {
    let app = require_app!();

    // The escalation: a read-only user could mint a token with full tenant
    // administration. Now making tokens takes `api_token_manage`.
    app.create_user_with_role("viewer", "read_only").await;
    let viewer = session(&app, "viewer").await;
    let refused = app
        .send_as(
            &viewer,
            Method::POST,
            "/api/v1/api-tokens",
            Some(json!({ "name": "mine", "permissions": ["admin"] })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    // And a read-only session no longer writes what nothing checked.
    for (method, path, body) in [
        (
            Method::POST,
            "/api/v1/flows",
            Some(json!({ "name": "f", "initial_node": "a", "nodes": {} })),
        ),
        (
            Method::POST,
            "/api/v1/voicemail/mailboxes",
            Some(json!({ "mailbox_id": "100" })),
        ),
        (
            Method::PUT,
            "/api/v1/config/call-manager/media",
            Some(json!({ "value": {} })),
        ),
    ] {
        let res = app.send_as(&viewer, method.clone(), path, body).await;
        assert_eq!(
            res.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            res.text
        );
    }
    let reads = app
        .send_as(&viewer, Method::GET, "/api/v1/flows", None)
        .await;
    assert_eq!(reads.status, StatusCode::OK, "{}", reads.text);

    // A tenant admin grants within the tenant, by name; `admin` is all of it.
    app.create_user_with_role("boss", "tenant_admin").await;
    let boss = session(&app, "boss").await;
    let grantable = app
        .send_as(&boss, Method::GET, "/api/v1/api-tokens/permissions", None)
        .await;
    assert_eq!(grantable.status, StatusCode::OK, "{}", grantable.text);
    let names: Vec<&str> = grantable
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"user_create") && names.contains(&"cdr_read"));
    assert!(!names.contains(&"system_config") && !names.contains(&"tenant_create"));
    for (perms, status) in [
        (json!(["system_config"]), StatusCode::FORBIDDEN),
        (json!(["read"]), StatusCode::BAD_REQUEST),
        (json!([]), StatusCode::BAD_REQUEST),
    ] {
        let res = app
            .send_as(
                &boss,
                Method::POST,
                "/api/v1/api-tokens",
                Some(json!({ "name": "x", "permissions": perms })),
            )
            .await;
        assert_eq!(res.status, status, "{perms}: {}", res.text);
    }

    // A scoped token reaches its routes and no others, and cannot make
    // tokens or rotate one that carries more than it does.
    let (_, cdrs) = make_token(&app, &boss, json!(["cdr_read", "api_token_manage"])).await;
    let ok = app.send_as(&cdrs, Method::GET, "/api/v1/cdrs", None).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    let flows = app.send_as(&cdrs, Method::GET, "/api/v1/flows", None).await;
    assert_eq!(flows.status, StatusCode::FORBIDDEN, "{}", flows.text);
    let spawn = app
        .send_as(
            &cdrs,
            Method::POST,
            "/api/v1/api-tokens",
            Some(json!({ "name": "child", "permissions": ["cdr_read"] })),
        )
        .await;
    assert_eq!(spawn.status, StatusCode::FORBIDDEN, "{}", spawn.text);
    let (users_id, users) = make_token(&app, &boss, json!(["user_read", "user_create"])).await;
    let steal = app
        .send_as(
            &cdrs,
            Method::POST,
            &format!("/api/v1/api-tokens/{users_id}/rotate"),
            None,
        )
        .await;
    assert_eq!(steal.status, StatusCode::FORBIDDEN, "{}", steal.text);

    // The maker answers for the token: demoted, the token loses what the
    // maker lost; disabled, the token is refused.
    let listed = app
        .send_as(&users, Method::GET, "/api/v1/users", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let demote = app
        .admin(
            Method::PUT,
            "/api/v1/users/boss",
            Some(json!({ "role": "read_only" })),
        )
        .await;
    assert_eq!(demote.status, StatusCode::OK, "{}", demote.text);
    let create = app
        .send_as(
            &users,
            Method::POST,
            "/api/v1/users",
            Some(json!({ "username": "ghost", "password": PASSWORD, "role": "end_user" })),
        )
        .await;
    assert_eq!(create.status, StatusCode::FORBIDDEN, "{}", create.text);
    let still_reads = app
        .send_as(&users, Method::GET, "/api/v1/users", None)
        .await;
    assert_eq!(still_reads.status, StatusCode::OK, "{}", still_reads.text);
    let disable = app
        .admin(
            Method::PUT,
            "/api/v1/users/boss",
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(disable.status, StatusCode::OK, "{}", disable.text);
    let gone = app
        .send_as(&users, Method::GET, "/api/v1/users", None)
        .await;
    assert_eq!(gone.status, StatusCode::UNAUTHORIZED, "{}", gone.text);

    // Rotation: the old secret works through its grace; with none, not at
    // all. Revoking keeps the row, marked.
    let (id, old) = {
        let res = app
            .admin(
                Method::POST,
                "/api/v1/api-tokens",
                Some(json!({ "name": "rotor", "permissions": ["device_read"] })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
        (
            res.json["token"]["id"].as_str().unwrap().to_string(),
            res.json["secret"].as_str().unwrap().to_string(),
        )
    };
    let rotated = app
        .admin(
            Method::POST,
            &format!("/api/v1/api-tokens/{id}/rotate"),
            Some(json!({ "grace_seconds": 600 })),
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    let new = rotated.json["secret"].as_str().unwrap().to_string();
    assert_ne!(new, old);
    assert!(!rotated.json["token"]["previous_secret_valid_until"].is_null());
    for secret in [&old, &new] {
        let res = app
            .send_as(secret, Method::GET, "/api/v1/devices", None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    }
    let again = app
        .admin(
            Method::POST,
            &format!("/api/v1/api-tokens/{id}/rotate"),
            Some(json!({ "grace_seconds": 0 })),
        )
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    let newest = again.json["secret"].as_str().unwrap().to_string();
    let stale = app
        .send_as(&new, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED, "{}", stale.text);
    let fresh = app
        .send_as(&newest, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text);
    let revoke = app
        .admin(Method::DELETE, &format!("/api/v1/api-tokens/{id}"), None)
        .await;
    assert_eq!(revoke.status, StatusCode::NO_CONTENT, "{}", revoke.text);
    let dead = app
        .send_as(&newest, Method::GET, "/api/v1/devices", None)
        .await;
    assert_eq!(dead.status, StatusCode::UNAUTHORIZED, "{}", dead.text);
    let list = app.admin(Method::GET, "/api/v1/api-tokens", None).await;
    let row = list
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .expect("a revoked token is kept")
        .clone();
    assert!(!row["revoked_at"].is_null(), "{row}");
    assert_eq!(row["revoked_by"], "static_token");

    // A personal token: its maker's /me routes, nothing else.
    app.create_user_with_role("pat", "end_user").await;
    let pat = session(&app, "pat").await;
    let made = app
        .send_as(
            &pat,
            Method::POST,
            "/api/v1/me/api-tokens",
            Some(json!({ "name": "crm" })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(made.json["token"]["scope"], "personal");
    let personal = made.json["secret"].as_str().unwrap().to_string();
    let personal_id = made.json["token"]["id"].as_str().unwrap().to_string();
    let profile = app
        .send_as(&personal, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(profile.status, StatusCode::OK, "{}", profile.text);
    assert_eq!(profile.json["username"], "pat");
    for (method, path, status) in [
        (Method::GET, "/api/v1/devices", StatusCode::FORBIDDEN),
        (Method::GET, "/api/v1/me/api-tokens", StatusCode::FORBIDDEN),
        // Not a session: it cannot be refreshed into one (refresh takes a
        // session's cookie, and nothing else).
        (
            Method::POST,
            "/api/v1/auth/refresh",
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let res = app.send_as(&personal, method.clone(), path, None).await;
        assert_eq!(res.status, status, "{method} {path}: {}", res.text);
    }
    // Nobody else's session sees it, and an admin cannot rotate it.
    let mine = app
        .send_as(&viewer, Method::GET, "/api/v1/me/api-tokens", None)
        .await;
    assert!(mine.json.as_array().unwrap().is_empty(), "{}", mine.text);
    let hijack = app
        .admin(
            Method::POST,
            &format!("/api/v1/api-tokens/{personal_id}/rotate"),
            None,
        )
        .await;
    assert_eq!(hijack.status, StatusCode::FORBIDDEN, "{}", hijack.text);
    // It can revoke itself.
    let bye = app
        .send_as(
            &personal,
            Method::DELETE,
            &format!("/api/v1/me/api-tokens/{personal_id}"),
            None,
        )
        .await;
    assert_eq!(bye.status, StatusCode::NO_CONTENT, "{}", bye.text);
    let after = app
        .send_as(&personal, Method::GET, "/api/v1/me/profile", None)
        .await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED, "{}", after.text);

    app.finish().await;
}

/// docs/DEVELOPER_SURFACE.md §3, phase 2: every error under /api/v1 is the
/// JSON error, whoever refused; every response carries a request id; a
/// browser may PATCH and name a tenant; a token's requests carry its limit.
#[tokio::test]
async fn every_error_is_json_and_every_response_has_an_id() {
    let app = require_app!();
    let is_json_error = |res: &Response, status: StatusCode, code: &str| {
        assert_eq!(res.status, status, "{}", res.text);
        assert_eq!(
            res.headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "{}",
            res.text
        );
        assert_eq!(res.json["code"], code, "{}", res.text);
        assert!(res.json["error"].is_string(), "{}", res.text);
        assert!(
            res.headers.get("x-request-id").is_some(),
            "no request id on {}",
            res.text
        );
    };

    // The bearer layer, the permission gate, an extractor, the router.
    let anonymous = app.send(Method::GET, "/api/v1/users", &[], None).await;
    is_json_error(&anonymous, StatusCode::UNAUTHORIZED, "unauthorized");
    app.create_user_with_role("peeker", "read_only").await;
    let peeker = session(&app, "peeker").await;
    let refused = app
        .send_as(&peeker, Method::DELETE, "/api/v1/users/peeker", None)
        .await;
    is_json_error(&refused, StatusCode::FORBIDDEN, "forbidden");
    let malformed = app
        .send_raw(
            ADMIN_TOKEN,
            Method::POST,
            "/api/v1/users",
            "application/json",
            b"{not json".to_vec(),
        )
        .await;
    is_json_error(&malformed, StatusCode::BAD_REQUEST, "bad_request");
    assert!(malformed.json["details"].is_string(), "{}", malformed.text);
    let wrong_method = app.admin(Method::PATCH, "/api/v1/users", None).await;
    is_json_error(
        &wrong_method,
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    );
    let bad_tenant = app
        .send(
            Method::GET,
            "/api/v1/users",
            &[(AUTH_HEADER, BEARER), ("x-fcp-tenant-id", "not-a-tenant")],
            None,
        )
        .await;
    is_json_error(&bad_tenant, StatusCode::BAD_REQUEST, "bad_request");

    // A caller's request id comes back; one is made when none is sent.
    let quoted = app
        .send(
            Method::GET,
            "/api/v1/users",
            &[(AUTH_HEADER, BEARER), ("x-request-id", "trace-me-42")],
            None,
        )
        .await;
    assert_eq!(quoted.status, StatusCode::OK, "{}", quoted.text);
    assert_eq!(quoted.headers.get("x-request-id").unwrap(), "trace-me-42");

    // A browser may PATCH and name a tenant.
    let preflight = app
        .send(
            Method::OPTIONS,
            "/api/v1/devices/dev-1",
            &[
                ("origin", "https://portal.example.com"),
                ("access-control-request-method", "PATCH"),
                (
                    "access-control-request-headers",
                    "authorization,x-fcp-tenant-id",
                ),
            ],
            None,
        )
        .await;
    assert!(preflight.status.is_success(), "{}", preflight.text);
    let methods = preflight
        .headers
        .get("access-control-allow-methods")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(methods.contains("PATCH"), "{methods}");
    let allowed = preflight
        .headers
        .get("access-control-allow-headers")
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(allowed.contains("x-fcp-tenant-id"), "{allowed}");

    // A token's requests carry its limit; the static token's do not.
    let token = app
        .admin(
            Method::POST,
            "/api/v1/api-tokens",
            Some(json!({ "name": "limited", "permissions": ["user_read"] })),
        )
        .await;
    let secret = token.json["secret"].as_str().unwrap().to_string();
    let first = app
        .send_as(&secret, Method::GET, "/api/v1/users", None)
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(first.headers.get("ratelimit-limit").unwrap(), "600");
    let second = app
        .send_as(&secret, Method::GET, "/api/v1/users", None)
        .await;
    let remaining = |r: &Response| -> u32 {
        r.headers
            .get("ratelimit-remaining")
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(remaining(&second) < remaining(&first));
    assert!(quoted.headers.get("ratelimit-limit").is_none());

    app.finish().await;
}

/// docs/DEVELOPER_SURFACE.md §3, phase 2c: every list is `{data,
/// next_cursor, total}` and pages by cursor, each row once.
#[tokio::test]
async fn every_list_pages_by_cursor() {
    let app = require_app!();
    for name in ["pa", "pb", "pc", "pd", "pe"] {
        app.create_user_with_role(name, "end_user").await;
    }

    // A list held whole: walked two at a time, in its order, each row once.
    let mut seen = Vec::new();
    let mut path = "/api/v1/users?limit=2".to_string();
    let mut pages = 0;
    loop {
        let page = app.admin(Method::GET, &path, None).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.text);
        assert!(page.body["data"].is_array(), "{}", page.text);
        assert!(page.json.as_array().unwrap().len() <= 2);
        let total = page.body["total"].as_u64().expect("a total");
        for u in page.json.as_array().unwrap() {
            seen.push(u["username"].as_str().unwrap().to_string());
        }
        pages += 1;
        match page.body["next_cursor"].as_str() {
            Some(c) => path = format!("/api/v1/users?limit=2&cursor={c}"),
            None => {
                assert_eq!(seen.len() as u64, total);
                break;
            }
        }
        assert!(pages < 20, "the walk ends");
    }
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen, sorted, "users come by username");
    sorted.dedup();
    assert_eq!(sorted.len(), seen.len(), "no row twice: {seen:?}");
    for name in ["pa", "pb", "pc", "pd", "pe"] {
        assert!(seen.iter().any(|s| s == name), "{name} missing: {seen:?}");
    }

    // A log paged by the store: the audit log, newest first, by keyset.
    // Audit rows are written after the response; wait for all five.
    for _ in 0..50 {
        let all = app
            .admin(Method::GET, "/api/v1/audit-log?limit=1000", None)
            .await;
        if all.json.as_array().unwrap().len() >= 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut ids = Vec::new();
    let mut times = Vec::new();
    let mut path = "/api/v1/audit-log?limit=2".to_string();
    loop {
        let page = app.admin(Method::GET, &path, None).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.text);
        for e in page.json.as_array().unwrap() {
            ids.push(e["id"].as_i64().unwrap());
            times.push(rfc3339_secs(&e["occurred_at"]));
        }
        match page.body["next_cursor"].as_str() {
            Some(c) => path = format!("/api/v1/audit-log?limit=2&cursor={c}"),
            None => break,
        }
        assert!(ids.len() < 200, "the walk ends");
    }
    assert!(ids.len() >= 5, "five user creations audited: {ids:?}");
    assert!(
        times.windows(2).all(|w| w[0] >= w[1]),
        "newest first: {times:?}"
    );
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "each row once: {ids:?}");

    // A limit or a cursor the API cannot use is a 400.
    for bad in [
        "/api/v1/users?limit=0",
        "/api/v1/users?limit=1001",
        "/api/v1/users?cursor=%25%25",
        "/api/v1/audit-log?cursor=bm90LWEta2V5",
    ] {
        let res = app.admin(Method::GET, bad, None).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{bad}: {}", res.text);
    }

    app.finish().await;
}

// ─── CDRs, usage, billing ────────────────────────────────────────────────

#[tokio::test]
async fn cdrs_usage_and_billing_summary() {
    let app = require_app!();
    let a1 = app
        .seed_cdr(
            app.tenant_a,
            Some(58_000),
            Some(rust_decimal_macros::dec!(0.25)),
        )
        .await;
    let _a2 = app
        .seed_cdr(
            app.tenant_a,
            Some(30_000),
            Some(rust_decimal_macros::dec!(0.10)),
        )
        .await;
    let _a3 = app.seed_cdr(app.tenant_a, None, None).await;
    let b1 = app
        .seed_cdr(
            app.tenant_b,
            Some(10_000),
            Some(rust_decimal_macros::dec!(1.00)),
        )
        .await;

    // The platform token is a cross-tenant reporting identity: with no
    // tenant selected it sees every tenant's records...
    let all = app.admin(Method::GET, "/api/v1/cdrs", None).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    assert_eq!(all.body["total"], 4, "{}", all.text);
    let by_param = app
        .admin(
            Method::GET,
            &format!("/api/v1/cdrs?tenant_id={}", app.tenant_b),
            None,
        )
        .await;
    assert_eq!(by_param.body["total"], 1, "{}", by_param.text);

    // ...and the override header scopes it like every other endpoint.
    let list = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/cdrs", None)
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.body["total"], 3, "{}", list.text);
    assert_eq!(list.json.as_array().unwrap().len(), 3);

    let answered = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/cdrs?disposition=answered&limit=1",
            None,
        )
        .await;
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.text);
    assert_eq!(answered.body["total"], 2, "{}", answered.text);
    assert_eq!(answered.json.as_array().unwrap().len(), 1, "limit applies");
    assert_eq!(answered.json[0]["disposition"], "answered");

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
    assert_eq!(b_list.body["total"], 1, "{}", b_list.text);

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

/// Where a carrier's calls come from, and which way a new trunk carries
/// calls (`docs/TENANT_TRUNKS.md` §7).
#[tokio::test]
async fn a_trunk_names_where_its_carriers_calls_come_from() {
    let app = require_app!(DeploymentMode::Carrier);

    let created = app
        .admin(
            Method::POST,
            "/api/v1/trunks",
            Some(json!({
                "name": "carrier",
                "hosts": ["sip.carrier.example.net"],
                "inbound_sources": ["203.0.113.0/24", " 198.51.100.7 ", ""],
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    assert_eq!(
        created.json["inbound_sources"],
        json!(["203.0.113.0/24", "198.51.100.7"]),
        "trimmed, blanks dropped"
    );
    // A new trunk carries calls both ways unless it says otherwise:
    // inbound admission honours direction now.
    assert_eq!(created.json["direction"], "bidirectional");
    let id = created.json["id"].as_str().unwrap().to_string();

    let got = app
        .admin(Method::GET, &format!("/api/v1/trunks/{id}"), None)
        .await;
    assert_eq!(got.json["inbound_sources"], created.json["inbound_sources"]);

    let refused = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/trunks/{id}"),
            Some(json!({ "inbound_sources": ["sip.carrier.example.net"] })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert!(
        refused.text.contains("not an IP address or CIDR range"),
        "{}",
        refused.text
    );

    let cleared = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/trunks/{id}"),
            Some(json!({ "inbound_sources": [] })),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text);
    assert_eq!(cleared.json["inbound_sources"], json!([]));

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

    // Messages deleted through the API, one by one or with their mailbox,
    // are in the deletion log under whoever asked, their audio queued.
    let (one, two) = (
        fcp_core::VoicemailMessage::new(id.clone(), "+15550002222".into(), 3, "/vm/one.wav".into()),
        fcp_core::VoicemailMessage::new(id.clone(), "+15550003333".into(), 4, "/vm/two.wav".into()),
    );
    {
        use fcp_core::storage::VoicemailStore;
        app.storage.put_message(one.clone()).await.unwrap();
        app.storage.put_message(two.clone()).await.unwrap();
    }
    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/voicemail/messages/{}", one.id),
            None,
        )
        .await;
    assert!(gone.status.is_success(), "{} {}", gone.status, gone.text);

    let del = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/voicemail/mailboxes/{id}"),
            None,
        )
        .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT, "{}", del.text);
    let log = app
        .admin(
            Method::GET,
            "/api/v1/compliance/deletions?class=voicemail",
            None,
        )
        .await;
    assert_eq!(log.status, StatusCode::OK, "{}", log.text);
    let entries = log.json.as_array().unwrap();
    let entry = |item: &str| {
        entries
            .iter()
            .find(|e| e["item"] == item)
            .unwrap_or_else(|| panic!("{item} not in {}", log.text))
            .clone()
    };
    assert_eq!(entry(&one.id)["cause"], "manual");
    assert_eq!(entry(&one.id)["actor"], "static_token");
    assert_eq!(entry(&one.id)["detail"]["path"], "/vm/one.wav");
    assert_eq!(entry(&two.id)["cause"], "cascade");
    assert_eq!(entry(&two.id)["actor"], "static_token");
    let queued: Vec<String> =
        sqlx::query_scalar("SELECT path FROM pending_file_deletions ORDER BY path")
            .fetch_all(app.storage.pool())
            .await
            .unwrap();
    assert_eq!(queued, vec!["/vm/one.wav", "/vm/two.wav"]);
    let other = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            "/api/v1/compliance/deletions?class=voicemail",
            None,
        )
        .await;
    assert_eq!(
        other.json.as_array().map(Vec::len),
        Some(0),
        "{}",
        other.text
    );
    // The audit event names what the request deleted.
    let audited = wait_for_audit(
        &app,
        "/api/v1/audit-log?action=DELETE%20/api/v1/voicemail/messages",
        |e| e["status"].as_u64().is_some_and(|s| s < 300),
    )
    .await;
    assert_eq!(
        audited["detail"]["params"]["id"],
        one.id.as_str(),
        "{audited}"
    );
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

#[tokio::test]
async fn legal_holds_are_placed_block_deletion_and_are_released() {
    let app = require_app!();
    let owner = app.create_user("held-owner", None).await;
    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550004444", "tenant_id": app.tenant_a })),
        )
        .await;
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
            })),
        )
        .await;
    assert_eq!(mailbox.status, StatusCode::CREATED, "{}", mailbox.text);
    let mailbox_id = mailbox.json["id"].as_str().unwrap().to_string();
    let message = fcp_core::VoicemailMessage::new(
        mailbox_id.clone(),
        "+1 415 555 0100".into(),
        3,
        "/vm/held.wav".into(),
    );
    {
        use fcp_core::storage::VoicemailStore;
        app.storage.put_message(message.clone()).await.unwrap();
    }

    // Validation: a matter, a real user, a window that is one.
    for bad in [
        json!({ "matter": " " }),
        json!({ "matter": "x", "subjects": [{ "type": "user", "username": "nobody" }] }),
        json!({ "matter": "x", "from": 20, "to": 10 }),
    ] {
        let refused = app
            .admin(Method::POST, "/api/v1/compliance/holds", Some(bad.clone()))
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{bad}: {}",
            refused.text
        );
    }

    // A hold on the caller, by another spelling of their number.
    let placed = app
        .admin(
            Method::POST,
            "/api/v1/compliance/holds",
            Some(json!({
                "matter": "Smith v. Example",
                "reason": "litigation",
                "subjects": [{ "type": "number", "number": "tel:+1-415-555-0100" }],
                "classes": ["voicemail", "cdr"],
            })),
        )
        .await;
    assert_eq!(placed.status, StatusCode::CREATED, "{}", placed.text);
    let hold_id = placed.json["id"].as_str().unwrap().to_string();
    assert!(hold_id.starts_with("hold-"));
    assert_eq!(placed.json["active"], true);
    assert!(placed.json["subject_keys"]
        .as_array()
        .unwrap()
        .contains(&json!("tail:4155550100")));
    let listed = app
        .admin(Method::GET, "/api/v1/compliance/holds", None)
        .await;
    assert_eq!(
        listed.json.as_array().map(Vec::len),
        Some(1),
        "{}",
        listed.text
    );
    let one = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/holds/{hold_id}"),
            None,
        )
        .await;
    assert_eq!(one.json["held"]["voicemail"], 1, "{}", one.text);
    assert_eq!(one.json["held"]["cdr"], 0, "{}", one.text);
    let other = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/compliance/holds/{hold_id}"),
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "another tenant's hold");

    // Deleting the message or its mailbox is refused. (Deleting the number
    // no longer reaches them: a mailbox outlives its line.)
    for path in [
        format!("/api/v1/voicemail/messages/{}", message.id),
        format!("/api/v1/voicemail/mailboxes/{mailbox_id}"),
    ] {
        let refused = app.admin(Method::DELETE, &path, None).await;
        assert_eq!(
            refused.status,
            StatusCode::CONFLICT,
            "{path}: {}",
            refused.text
        );
        assert!(refused.text.contains(&hold_id), "{}", refused.text);
    }
    let tenant_gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            None,
        )
        .await;
    assert_eq!(
        tenant_gone.status,
        StatusCode::CONFLICT,
        "{}",
        tenant_gone.text
    );

    // Released with a reason; the message can go.
    let no_reason = app
        .admin(
            Method::POST,
            &format!("/api/v1/compliance/holds/{hold_id}/release"),
            Some(json!({ "reason": "" })),
        )
        .await;
    assert_eq!(no_reason.status, StatusCode::BAD_REQUEST);
    let released = app
        .admin(
            Method::POST,
            &format!("/api/v1/compliance/holds/{hold_id}/release"),
            Some(json!({ "reason": "settled" })),
        )
        .await;
    assert_eq!(released.status, StatusCode::OK, "{}", released.text);
    assert_eq!(released.json["active"], false);
    assert_eq!(released.json["released_by"], "static_token");
    let again = app
        .admin(
            Method::POST,
            &format!("/api/v1/compliance/holds/{hold_id}/release"),
            Some(json!({ "reason": "settled" })),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/voicemail/messages/{}", message.id),
            None,
        )
        .await;
    assert!(gone.status.is_success(), "{} {}", gone.status, gone.text);
    let all = app
        .admin(Method::GET, "/api/v1/compliance/holds?all=true", None)
        .await;
    assert_eq!(all.json[0]["release_reason"], "settled", "{}", all.text);
    let active = app
        .admin(Method::GET, "/api/v1/compliance/holds", None)
        .await;
    assert_eq!(active.json.as_array().map(Vec::len), Some(0));

    // Holds take their own permission.
    app.create_user_with_role("clerk", "user_admin").await;
    let clerk = app.login("clerk", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = app
        .send_as(&clerk, Method::GET, "/api/v1/compliance/holds", None)
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

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

    // `variables` is optional.
    let mut bare = hangup_flow("bare", "No variables");
    bare.as_object_mut().unwrap().remove("variables");
    let created = app.admin(Method::POST, "/api/v1/flows", Some(bare)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    assert_eq!(created.json["variables"], json!({}));
    app.admin(Method::DELETE, "/api/v1/flows/bare", None).await;

    let list = app.admin(Method::GET, "/api/v1/flows", None).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.body["total"], 1, "{}", list.text);
    assert_eq!(list.json[0]["id"], "main-ivr");

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
    assert_eq!(updated.json["version"], 2, "the version it was stored as");

    // A second edit, of the nodes: every version is kept, as it was.
    let edited = app
        .admin(
            Method::PUT,
            "/api/v1/flows/main-ivr",
            Some(json!({ "nodes": {
                "answer": { "type": "answer", "next": "bye" },
                "bye": { "type": "hangup", "reason": "edited" }
            }})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.text);
    assert_eq!(edited.json["version"], 3);

    let versions = app
        .admin(Method::GET, "/api/v1/flows/main-ivr/versions", None)
        .await;
    assert_eq!(versions.status, StatusCode::OK, "{}", versions.text);
    let numbers: Vec<_> = versions
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, vec![3, 2, 1], "{}", versions.text);

    let first = app
        .admin(Method::GET, "/api/v1/flows/main-ivr/versions/1", None)
        .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(first.json["name"], "Main IVR");
    assert_eq!(first.json["enabled"], true);
    assert_eq!(first.json["nodes"]["hangup"]["type"], "hangup");
    assert!(first.json["nodes"].get("bye").is_none(), "{}", first.text);
    let second = app
        .admin(Method::GET, "/api/v1/flows/main-ivr/versions/2", None)
        .await;
    assert_eq!(second.json["name"], "Main IVR v2");
    assert!(second.json["nodes"].get("bye").is_none(), "{}", second.text);
    assert_eq!(
        app.admin(Method::GET, "/api/v1/flows/main-ivr/versions/9", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.admin_in(
            app.tenant_b,
            Method::GET,
            "/api/v1/flows/main-ivr/versions/1",
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

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
    assert_eq!(b_list.body["total"], 0, "{}", b_list.text);

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
    // Its versions went with it.
    assert_eq!(
        app.admin(Method::GET, "/api/v1/flows/main-ivr/versions/1", None)
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
    assert_eq!(list.json.as_array().unwrap().len(), 2, "{}", list.text);
    assert!(list
        .json
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
    assert_eq!(
        filtered.json.as_array().unwrap().len(),
        1,
        "{}",
        filtered.text
    );
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
    assert_eq!(all.json.as_array().unwrap().len(), 3, "{}", all.text);

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

/// A supervisor on a call (`docs/ENTERPRISE_CALLING.md` §3.11): the
/// routes are an admin's (`call_supervise`), and with no call manager to
/// ask they say so rather than pretend.
#[tokio::test]
async fn supervising_a_call_is_an_admins_and_needs_a_call_manager() {
    let app = require_app!();
    app.create_user_with_role("agent", "end_user").await;
    let call = "call-00000000-0000-0000-0000-000000000001";
    let body = json!({ "supervisor": "1001" });
    let asked = app
        .admin(
            Method::POST,
            &format!("/api/v1/calls/{call}/monitor"),
            Some(body.clone()),
        )
        .await;
    assert_eq!(
        asked.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        asked.text
    );
    let empty = app
        .admin(
            Method::POST,
            &format!("/api/v1/calls/{call}/barge"),
            Some(json!({ "supervisor": " " })),
        )
        .await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);
    let agent = app.login("agent", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    for (method, path) in [
        (Method::POST, format!("/api/v1/calls/{call}/whisper")),
        (Method::GET, format!("/api/v1/calls/{call}/supervisor")),
        (Method::DELETE, format!("/api/v1/calls/{call}/supervisor")),
    ] {
        let refused = app.send_as(&agent, method, &path, Some(body.clone())).await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{path}: {}",
            refused.text
        );
    }
    app.finish().await;
}

/// Executive-assistant (`docs/ENTERPRISE_CALLING.md` §3.10): an admin
/// names an executive's assistants, the executive sets their own handling
/// but not who assists them, and the assistant sees and opts out of the
/// executives they assist.
#[tokio::test]
async fn an_admin_names_assistants_and_the_assistant_opts_out() {
    let app = require_app!();
    let boss_id = app.create_user_with_role("boss", "end_user").await;
    let aide_id = app.create_user_with_role("aide", "end_user").await;

    let set = app
        .admin(
            Method::PUT,
            "/api/v1/users/boss/calling",
            Some(json!({ "executive": {
                "enabled": true,
                "filter": "external",
                "assistants": ["aide"],
            } })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(set.json["executive"]["filter"], "external");
    assert_eq!(set.json["executive"]["assistants"][0]["user_id"], aide_id);
    assert_eq!(set.json["executive"]["assistants"][0]["opted_in"], true);

    // Nobody who is not a user, nor the executive themself.
    for bad in [json!(["nobody"]), json!(["boss"])] {
        let refused = app
            .admin(
                Method::PUT,
                "/api/v1/users/boss/calling",
                Some(json!({ "executive": { "assistants": bad } })),
            )
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    }

    // The executive turns it off, but cannot name assistants.
    let boss = app.login("boss", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let off = app
        .send_as(
            &boss,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "executive": { "enabled": false, "alert": "assistants_and_executive" } })),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.text);
    assert_eq!(off.json["executive"]["enabled"], false);
    assert_eq!(off.json["executive"]["alert"], "assistants_and_executive");
    assert_eq!(off.json["executive"]["assistants"][0]["user_id"], aide_id);
    let naming = app
        .send_as(
            &boss,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "executive": { "assistants": [] } })),
        )
        .await;
    assert_eq!(naming.status, StatusCode::FORBIDDEN, "{}", naming.text);

    // The assistant sees the executive and opts out.
    let aide = app.login("aide", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let mine = app
        .send_as(&aide, Method::GET, "/api/v1/me/executives", None)
        .await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.text);
    assert_eq!(mine.json[0]["user_id"], boss_id);
    assert_eq!(mine.json[0]["username"], "boss");
    assert_eq!(mine.json[0]["opted_in"], true);
    let out = app
        .send_as(
            &aide,
            Method::PUT,
            &format!("/api/v1/me/executives/{boss_id}"),
            Some(json!({ "opted_in": false })),
        )
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT, "{}", out.text);
    let seen = app
        .admin(Method::GET, "/api/v1/users/boss/calling", None)
        .await;
    assert_eq!(seen.json["executive"]["assistants"][0]["opted_in"], false);
    // Re-naming the assistants keeps the assistant's own choice.
    let renamed = app
        .admin(
            Method::PUT,
            "/api/v1/users/boss/calling",
            Some(json!({ "executive": { "assistants": [aide_id] } })),
        )
        .await;
    assert_eq!(
        renamed.json["executive"]["assistants"][0]["opted_in"],
        false
    );
    // Not an assistant of someone else.
    let stranger = app
        .send_as(
            &boss,
            Method::PUT,
            &format!("/api/v1/me/executives/{aide_id}"),
            Some(json!({ "opted_in": true })),
        )
        .await;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND, "{}", stranger.text);

    app.finish().await;
}

#[tokio::test]
async fn self_service_profile_password_calling_and_voicemail() {
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

    // The user's own calling settings: the defaults until set, then what
    // they set, which the admin sees too (docs/ENTERPRISE_CALLING.md §3.1).
    let calling = app
        .send_as(&jwt, Method::GET, "/api/v1/me/calling", None)
        .await;
    assert_eq!(calling.status, StatusCode::OK, "{}", calling.text);
    assert_eq!(calling.json["username"], "selfie");
    assert_eq!(calling.json["dnd"], false);
    assert_eq!(calling.json["call_waiting"], true);
    assert!(calling.json["no_answer_secs"].is_null(), "{}", calling.text);
    assert!(
        calling.json["forward_no_answer"].is_null(),
        "{}",
        calling.text
    );
    let set_calling = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "forward_no_answer": "+15559990000", "forward_busy": "voicemail", "no_answer_secs": 15, "dnd": true })),
        )
        .await;
    assert_eq!(set_calling.status, StatusCode::OK, "{}", set_calling.text);
    assert_eq!(set_calling.json["forward_no_answer"], "number:+15559990000");
    assert_eq!(set_calling.json["forward_busy"], "voicemail");
    assert_eq!(set_calling.json["no_answer_secs"], 15);
    assert_eq!(set_calling.json["dnd"], true);
    let cleared = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "forward_busy": "" })),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text);
    assert!(cleared.json["forward_busy"].is_null(), "{}", cleared.text);
    assert_eq!(cleared.json["forward_no_answer"], "number:+15559990000");
    let bad = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "forward_all": "not a target", "no_answer_secs": 2 })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let as_admin = app
        .admin(Method::GET, "/api/v1/users/selfie/calling", None)
        .await;
    assert_eq!(as_admin.status, StatusCode::OK, "{}", as_admin.text);
    assert_eq!(as_admin.json["dnd"], true);
    assert_eq!(as_admin.json["no_answer_secs"], 15);
    let admin_set = app
        .admin(
            Method::PUT,
            "/api/v1/users/selfie/calling",
            Some(json!({ "dnd": false })),
        )
        .await;
    assert_eq!(admin_set.status, StatusCode::OK, "{}", admin_set.text);
    assert_eq!(admin_set.json["dnd"], false);
    // Hold music: silence, back to the tenant's, and only a real prompt.
    let silent = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "hold_music": "none" })),
        )
        .await;
    assert_eq!(silent.status, StatusCode::OK, "{}", silent.text);
    assert_eq!(silent.json["hold_music"], "none");
    let tenants = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "hold_music": "" })),
        )
        .await;
    assert_eq!(tenants.status, StatusCode::OK, "{}", tenants.text);
    assert!(tenants.json["hold_music"].is_null(), "{}", tenants.text);
    let unknown = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "hold_music": "prompt-00000000-0000-0000-0000-000000000000" })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.text);

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

/// A tenant's networks place the devices that move, and a user says where
/// they are when nothing else does (`docs/E911_BLOCK_2.md` §6).
#[tokio::test]
async fn e911_networks_and_declared_locations() {
    let app = require_app!();

    let site = app
        .admin(
            Method::POST,
            "/api/v1/locations",
            Some(json!({
                "name": "Branch",
                "address_line1": "9 Elm St",
                "city": "Troy",
                "state": "NY",
                "postal_code": "12180",
                "country": "US",
                "e911_callback_number": "+15185550222"
            })),
        )
        .await;
    assert_eq!(site.status, StatusCode::CREATED, "{}", site.text);
    let site_id = site.json["id"].as_str().unwrap().to_string();

    // A network is mapped to that site, for the devices that move.
    let created = app
        .admin(
            Method::POST,
            "/api/v1/emergency/networks",
            Some(json!({ "cidr": "10.1.2.0/24", "location_id": site_id, "name": "Branch wired" })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    assert_eq!(created.json["cidr"], "10.1.2.0/24");
    assert_eq!(created.json["applies_to"], "nomadic", "{}", created.text);
    let network_id = created.json["id"].as_str().unwrap().to_string();

    // A single address is a network of one.
    let single = app
        .admin(
            Method::POST,
            "/api/v1/emergency/networks",
            Some(json!({ "cidr": "10.9.9.9", "location_id": site_id, "applies_to": "all" })),
        )
        .await;
    assert_eq!(single.status, StatusCode::CREATED, "{}", single.text);
    assert_eq!(single.json["cidr"], "10.9.9.9/32", "{}", single.text);

    // Nonsense, an unknown rule and a site of another tenant are refused.
    for body in [
        json!({ "cidr": "not-a-network", "location_id": site_id }),
        json!({ "cidr": "10.0.0.0/8", "location_id": site_id, "applies_to": "sometimes" }),
        json!({ "cidr": "10.0.0.0/8", "location_id": fcp_core::LocationId::new().to_string() }),
    ] {
        let refused = app
            .admin(
                Method::POST,
                "/api/v1/emergency/networks",
                Some(body.clone()),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{body}: {}",
            refused.text
        );
    }

    let listed = app
        .admin(Method::GET, "/api/v1/emergency/networks", None)
        .await;
    assert_eq!(listed.json.as_array().unwrap().len(), 2, "{}", listed.text);

    let patched = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/emergency/networks/{network_id}"),
            Some(json!({ "applies_to": "all", "priority": 5 })),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text);
    assert_eq!(patched.json["applies_to"], "all");
    assert_eq!(patched.json["priority"], 5);

    let deleted = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/emergency/networks/{network_id}"),
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let gone = app
        .admin(
            Method::GET,
            &format!("/api/v1/emergency/networks/{network_id}"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);

    // Which device types may send their own location, and how long a
    // declaration stands.
    let config = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({
                "trust_device_locations": ["conference_room"],
                "declared_location_ttl_secs": 7200
            })),
        )
        .await;
    assert_eq!(config.status, StatusCode::OK, "{}", config.text);
    assert_eq!(config.json["trust_device_locations"][0], "conference_room");
    assert_eq!(config.json["declared_location_ttl_secs"], 7200);
    let refused = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({ "trust_device_locations": ["toaster"] })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    // A user says where they are; it is validated, dated and expires.
    app.create_user_with_role("nomad", "end_user").await;
    let jwt = app.login("nomad", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let none_yet = app
        .send_as(&jwt, Method::GET, "/api/v1/me/location", None)
        .await;
    assert_eq!(none_yet.status, StatusCode::NOT_FOUND, "{}", none_yet.text);

    let declared = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/location",
            Some(json!({
                "address1": "500 Hotel Rd",
                "city": "Saratoga Springs",
                "state": "NY",
                "postal_code": "12866",
                "house_number": "500",
                "street_name": "Hotel",
                "street_suffix": "Rd"
            })),
        )
        .await;
    assert_eq!(declared.status, StatusCode::OK, "{}", declared.text);
    assert_eq!(declared.json["address"]["city"], "Saratoga Springs");
    assert_eq!(declared.json["address"]["street_name"], "Hotel");
    assert_eq!(declared.json["address"]["validated"], true);
    assert_eq!(declared.json["current"], true);
    let declared_at = rfc3339_secs(&declared.json["declared_at"]);
    let expires_at = rfc3339_secs(&declared.json["expires_at"]);
    assert_eq!(expires_at - declared_at, 7200, "the tenant's TTL");

    // An address the validator refuses is not stored.
    let bad = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/location",
            Some(json!({
                "address1": "Hotel Rd",
                "city": "",
                "state": "New York",
                "postal_code": "1"
            })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", bad.text);
    assert!(
        !bad.json["issues"].as_array().unwrap().is_empty(),
        "{}",
        bad.text
    );

    let mine = app
        .send_as(&jwt, Method::GET, "/api/v1/me/location", None)
        .await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.text);
    assert_eq!(mine.json["address"]["address1"], "500 Hotel Rd");

    let cleared = app
        .send_as(&jwt, Method::DELETE, "/api/v1/me/location", None)
        .await;
    assert_eq!(cleared.status, StatusCode::NO_CONTENT, "{}", cleared.text);
    let gone = app
        .send_as(&jwt, Method::GET, "/api/v1/me/location", None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);

    // The platform token has no user behind it.
    let no_user = app.admin(Method::GET, "/api/v1/me/location", None).await;
    assert_eq!(no_user.status, StatusCode::UNAUTHORIZED, "{}", no_user.text);
}

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
    // The free text a tenant saved is typed by its shape, and the plain
    // strings stay for clients written before block 2
    // (`docs/E911_BLOCK_2.md` §8.1).
    assert_eq!(
        reread.json["contacts"][0]["kind"], "email",
        "{}",
        reread.text
    );
    assert_eq!(reread.json["contacts"][0]["target"], "security@example.com");
    assert_eq!(reread.json["contacts"][1]["kind"], "call");

    // Contacts can be given typed, with a name, and are checked.
    let typed = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({ "contacts": [
                { "kind": "email", "target": "security@example.com", "name": "Front desk" },
                { "kind": "sms", "target": "+15185550143" },
                "2001"
            ]})),
        )
        .await;
    assert_eq!(typed.status, StatusCode::OK, "{}", typed.text);
    assert_eq!(typed.json["contacts"][0]["name"], "Front desk");
    assert_eq!(typed.json["contacts"][1]["kind"], "sms");
    assert_eq!(
        typed.json["contacts"][2]["kind"], "call",
        "a bare extension is a call: {}",
        typed.text
    );
    assert_eq!(
        typed.json["notify_contacts"],
        json!(["security@example.com", "+15185550143", "2001"])
    );
    let refused = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({ "contacts": [{ "kind": "email", "target": "nobody" }] })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    let unreadable = app
        .admin(
            Method::PUT,
            "/api/v1/emergency/config",
            Some(json!({ "contacts": ["down the hall"] })),
        )
        .await;
    assert_eq!(
        unreadable.status,
        StatusCode::BAD_REQUEST,
        "{}",
        unreadable.text
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
                "e911_callback_number": "+15185550100",
                "civic": { "house_number": "1", "street_name": "State", "street_suffix": "St" }
            })),
        )
        .await;
    assert_eq!(site.status, StatusCode::CREATED, "{}", site.text);
    let site_id = site.json["id"].as_str().unwrap().to_string();
    assert_eq!(site.json["e911_callback_number"], "+15185550100");
    assert_eq!(site.json["civic"]["street_name"], "State", "{}", site.text);

    // Civic elements are replaced on update and stored; bad ones are refused.
    let restructured = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/locations/{site_id}"),
            Some(
                json!({ "civic": { "house_number": "1", "street_name": "State",
                                    "street_suffix": "St", "unit": "300" } }),
            ),
        )
        .await;
    assert_eq!(restructured.status, StatusCode::OK, "{}", restructured.text);
    let fetched = app
        .admin(Method::GET, &format!("/api/v1/locations/{site_id}"), None)
        .await;
    assert_eq!(fetched.json["civic"]["unit"], "300", "{}", fetched.text);
    assert_eq!(
        fetched.json["civic"]["house_number"], "1",
        "{}",
        fetched.text
    );
    let refused = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/locations/{site_id}"),
            Some(json!({ "civic": { "street_name": "State\nX-Injected: 1" } })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

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
    // Until the site's address is validated the user is not fully ready.
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

    // Re-sending the same address keeps its validation; changing it clears
    // it, and a client cannot claim validation for itself.
    let unchanged = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/phone-numbers/{number_id}"),
            Some(json!({ "e911_address": {
                "address1": "2 Main St", "city": "Albany", "state": "NY",
                "postal_code": "12207", "country": "US", "validated": false,
                "house_number": "2", "street_name": "Main", "street_suffix": "St"
            }})),
        )
        .await;
    assert_eq!(unchanged.status, StatusCode::OK, "{}", unchanged.text);
    assert_eq!(
        unchanged.json["e911_address"]["validated"], true,
        "the same address keeps its validation: {}",
        unchanged.text
    );
    let moved = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/phone-numbers/{number_id}"),
            Some(json!({ "e911_address": {
                "address1": "3 Main St", "city": "Albany", "state": "NY",
                "postal_code": "12207", "country": "US", "validated": true
            }})),
        )
        .await;
    assert_eq!(
        moved.json["e911_address"]["validated"], false,
        "editing the address clears it, and `validated: true` is not believed: {}",
        moved.text
    );

    // A site's address is validated the same way, and a validated site
    // address makes the users placed there ready.
    let no_address = app
        .admin(
            Method::POST,
            "/api/v1/locations",
            Some(json!({ "name": "Nameplate only" })),
        )
        .await;
    let no_address_id = no_address.json["id"].as_str().unwrap().to_string();
    let refused = app
        .admin(
            Method::POST,
            &format!("/api/v1/locations/{no_address_id}/e911/validate"),
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    let site_check = app
        .admin(
            Method::POST,
            &format!("/api/v1/locations/{site_id}/e911/validate"),
            None,
        )
        .await;
    assert_eq!(site_check.status, StatusCode::OK, "{}", site_check.text);
    assert_eq!(site_check.json["validated"], true, "{}", site_check.text);
    assert_eq!(site_check.json["validation"]["validator"], "basic");
    let stored_site = app
        .admin(Method::GET, &format!("/api/v1/locations/{site_id}"), None)
        .await;
    assert_eq!(
        stored_site.json["e911_validated"], true,
        "{}",
        stored_site.text
    );
    assert_eq!(stored_site.json["e911_validated_by"], "basic");
    assert_eq!(
        stored_site.json["civic"]["unit"], "300",
        "the elements the tenant set are kept: {}",
        stored_site.text
    );
    assert!(
        stored_site.json["civic"]["floor"].is_null(),
        "the floor stays the site's own field: {}",
        stored_site.text
    );

    let after = app
        .admin(Method::GET, "/api/v1/emergency/readiness", None)
        .await;
    let ready = after.json["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "ready")
        .unwrap()
        .clone();
    assert_eq!(ready["ready"], true, "{ready}");
    assert!(ready["issues"].as_array().unwrap().is_empty(), "{ready}");
    assert_eq!(ready["location"]["address"]["validated"], true, "{ready}");

    // Editing any part of the address clears the site's validation again.
    let edited = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/locations/{site_id}"),
            Some(json!({ "address_line1": "2 State St" })),
        )
        .await;
    assert_eq!(edited.json["e911_validated"], false, "{}", edited.text);
    let renamed = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/locations/{site_id}"),
            Some(json!({ "description": "Head office" })),
        )
        .await;
    assert_eq!(
        renamed.json["e911_validated"], false,
        "nothing re-validates on its own: {}",
        renamed.text
    );

    // A device's own override is validated on the same route.
    let device = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({
                "name": "Lobby phone",
                "device_type": "desk_phone",
                "tenant_id": app.tenant_a,
                "mac_address": "aa-bb-cc-dd-ee-91",
                "e911_override": {
                    "address1": "1455 market street", "city": "San Francisco",
                    "state": "ca", "postal_code": "94103", "country": "US",
                    "validated": true
                }
            })),
        )
        .await;
    assert_eq!(device.status, StatusCode::CREATED, "{}", device.text);
    let device_id = device.json["id"].as_str().unwrap().to_string();
    assert_eq!(
        device.json["e911_override"]["validated"], false,
        "an address arrives unvalidated whatever it claims: {}",
        device.text
    );
    let checked = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{device_id}/e911/validate"),
            None,
        )
        .await;
    assert_eq!(checked.status, StatusCode::OK, "{}", checked.text);
    assert_eq!(checked.json["validated"], true, "{}", checked.text);
    assert_eq!(
        checked.json["validation"]["normalized"]["street_name"], "market",
        "the street is split into elements, keeping the case it was typed in: {}",
        checked.text
    );
    let stored_device = app
        .admin(Method::GET, &format!("/api/v1/devices/{device_id}"), None)
        .await;
    assert_eq!(
        stored_device.json["e911_override"]["validated"], true,
        "{}",
        stored_device.text
    );
    assert_eq!(
        stored_device.json["e911_override"]["validated_by"], "basic",
        "{}",
        stored_device.text
    );
    assert_eq!(
        stored_device.json["e911_override"]["address1"], "1455 market street",
        "the normalised address is stored: {}",
        stored_device.text
    );
    let device_moved = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
            Some(json!({ "e911_override": {
                "address1": "1500 Market St", "city": "San Francisco",
                "state": "CA", "postal_code": "94103", "country": "US"
            }})),
        )
        .await;
    assert_eq!(
        device_moved.json["e911_override"]["validated"], false,
        "moving the device clears its validation: {}",
        device_moved.text
    );
    // A phone that goes back to its site drops the address it carried:
    // a stale override would dispatch to where it used to be.
    let trusted = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
            Some(json!({ "e911_trust_location": true })),
        )
        .await;
    assert_eq!(
        trusted.json["e911_trust_location"], true,
        "{}",
        trusted.text
    );
    let cleared_device = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
            Some(json!({ "e911_override": null, "e911_trust_location": null })),
        )
        .await;
    assert!(
        cleared_device.json["e911_override"].is_null(),
        "the override is removed: {}",
        cleared_device.text
    );
    assert!(
        cleared_device.json["e911_trust_location"].is_null(),
        "and the device goes back to what its type is trusted with: {}",
        cleared_device.text
    );
    let untouched = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
            Some(json!({ "name": "Lobby phone" })),
        )
        .await;
    assert!(
        untouched.json["e911_override"].is_null(),
        "a change that says nothing about the address leaves it alone: {}",
        untouched.text
    );
    let bare_device = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({
                "name": "No override",
                "device_type": "desk_phone",
                "tenant_id": app.tenant_a,
                "mac_address": "aa-bb-cc-dd-ee-92"
            })),
        )
        .await;
    let bare_device_id = bare_device.json["id"].as_str().unwrap().to_string();
    let nothing = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{bare_device_id}/e911/validate"),
            None,
        )
        .await;
    assert_eq!(nothing.status, StatusCode::BAD_REQUEST, "{}", nothing.text);

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

    // One trunk model (`docs/TENANT_TRUNKS.md` §4): the API's flat JSON is
    // what it always was, while the model groups these into the transport,
    // the credentials, the digit plan and the media settings.
    let full = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/trunks/{trunk_id}"),
            Some(json!({
                "transport_protocol": "tls", "transport_port": 5061, "verify_tls": false,
                "auth_username": "fcp", "auth_password": "s3cret", "auth_realm": "carrier",
                "strip_digits": 2, "prepend_digits": "+1",
                "srtp_mode": "required", "dtmf_mode": "rfc2833",
                "codec_prefs": ["PCMU", "G722"], "header_rules": "strip-pai, add-diversion",
                "max_failures": 7, "recovery_interval_secs": 120,
                "register": true, "register_expiry": 600, "cps_limit": 5
            })),
        )
        .await;
    assert_eq!(full.status, StatusCode::OK, "{}", full.text);
    let stored_trunk = app
        .admin(Method::GET, &format!("/api/v1/trunks/{trunk_id}"), None)
        .await;
    let t = &stored_trunk.json;
    assert_eq!(t["transport_protocol"], "tls", "{}", stored_trunk.text);
    assert_eq!(t["transport_port"], 5061);
    assert_eq!(t["verify_tls"], false);
    assert_eq!(t["has_credentials"], true);
    assert_eq!(t["auth_username"], "fcp");
    assert_eq!(t["auth_realm"], "carrier");
    assert!(
        t.get("auth_password").is_none(),
        "the password never comes back: {}",
        stored_trunk.text
    );
    assert_eq!(t["strip_digits"], 2);
    assert_eq!(t["prepend_digits"], "+1");
    assert_eq!(t["srtp_mode"], "required");
    assert_eq!(t["dtmf_mode"], "rfc2833");
    assert_eq!(t["codec_prefs"], json!(["PCMU", "G722"]));
    assert_eq!(t["header_rules"], "strip-pai, add-diversion");
    assert_eq!(t["max_failures"], 7);
    assert_eq!(t["recovery_interval_secs"], 120);
    assert_eq!(t["register"], true);
    assert_eq!(t["register_expiry"], 600);
    assert_eq!(t["cps_limit"], 5);

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
        trunk_id: None,
        outcome: "placed".to_string(),
        placed_at: SystemTime::now(),
        notified: false,
        sip_status: None,
        answered_at: None,
        ended_at: None,
        delivery: Some("elin".to_string()),
        location_structured: Some(false),
        geolocation_error: None,
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
    assert_eq!(calls.json[0]["delivery"], "elin", "{}", calls.text);
    assert_eq!(calls.json[0]["outcome"], "placed", "{}", calls.text);
    assert!(calls.json[0]["ended_at"].is_null(), "{}", calls.text);

    // Completed when the call ends: the answering trunk, the status, the
    // times.
    let answered = SystemTime::now();
    app.emergency
        .complete_emergency_call(
            id,
            &fcp_core::e911::EmergencyCompletion {
                trunk_name: Some("e911-backup".to_string()),
                trunk_id: Some(fcp_core::TrunkId::from_string("trunk-e911-backup".into())),
                outcome: "answered".to_string(),
                sip_status: Some(200),
                answered_at: Some(answered),
                ended_at: answered + std::time::Duration::from_secs(42),
                delivery: Some("elin".to_string()),
                geolocation_error: Some(
                    "300;code=\"Insufficient Location Information\"".to_string(),
                ),
            },
        )
        .await
        .unwrap();
    let calls = app
        .admin(Method::GET, "/api/v1/emergency/calls", None)
        .await;
    assert_eq!(calls.json[0]["outcome"], "answered", "{}", calls.text);
    assert_eq!(calls.json[0]["sip_status"], 200, "{}", calls.text);
    assert_eq!(calls.json[0]["trunk_name"], "e911-backup", "{}", calls.text);
    assert_eq!(
        calls.json[0]["trunk_id"], "trunk-e911-backup",
        "names are unique only within a tenant, so the record says which trunk: {}",
        calls.text
    );
    assert!(!calls.json[0]["answered_at"].is_null(), "{}", calls.text);
    assert!(!calls.json[0]["ended_at"].is_null(), "{}", calls.text);
    assert_eq!(calls.json[0]["duration_secs"], 42, "{}", calls.text);
    assert_eq!(
        calls.json[0]["location_structured"], false,
        "{}",
        calls.text
    );
    assert_eq!(
        calls.json[0]["geolocation_error"], "300;code=\"Insufficient Location Information\"",
        "{}",
        calls.text
    );

    // Notification attempts are listed under the call they told someone
    // about (§8.3), and only for the tenant whose call it was.
    let notice = fcp_core::e911::notify::EmergencyNotice {
        tenant_id: app.tenant_a,
        call_id: record.call_id.clone(),
        kind: EmergencyKind::Live,
        dialed: "911".to_string(),
        caller: format!("sip:ready@{REALM}"),
        caller_name: Some("ready".to_string()),
        callback_number: Some("+15185550100".to_string()),
        location_summary: Some("HQ: 1 State St, Albany, NY 12207".to_string()),
        location_source: Some("user_location".to_string()),
        client_address: None,
        trunk_name: Some("e911-trunk".to_string()),
        placed_at: SystemTime::now(),
    };
    let mut delivered = fcp_core::e911::notify::EmergencyNotification::first(
        app.tenant_a,
        Some(id),
        record.call_id.clone(),
        fcp_core::e911::notify::NotifyContact::new(
            fcp_core::e911::notify::ContactKind::Email,
            "security@example.com",
        ),
        notice.clone(),
    );
    delivered.status = fcp_core::e911::notify::NotificationStatus::Delivered;
    delivered.delivered_at = Some(SystemTime::now());
    app.emergency
        .insert_emergency_notification(&delivered)
        .await
        .unwrap();
    let mut failed = delivered.next_attempt();
    failed.contact = fcp_core::e911::notify::NotifyContact::new(
        fcp_core::e911::notify::ContactKind::Sms,
        "+15185550143",
    );
    failed.status = fcp_core::e911::notify::NotificationStatus::Failed;
    failed.error = Some("provider down".to_string());
    app.emergency
        .insert_emergency_notification(&failed)
        .await
        .unwrap();

    let attempts = app
        .admin(
            Method::GET,
            &format!("/api/v1/emergency/calls/{}/notifications", record.call_id),
            None,
        )
        .await;
    assert_eq!(attempts.status, StatusCode::OK, "{}", attempts.text);
    let rows = attempts.json.as_array().unwrap();
    assert_eq!(rows.len(), 2, "{}", attempts.text);
    assert_eq!(rows[0]["kind"], "email");
    assert_eq!(rows[0]["status"], "delivered");
    assert_eq!(rows[1]["kind"], "sms");
    assert_eq!(rows[1]["status"], "failed");
    assert_eq!(rows[1]["error"], "provider down");
    assert_eq!(rows[1]["attempt"], 2);
    let other_tenant = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/emergency/calls/{}/notifications", record.call_id),
            None,
        )
        .await;
    assert!(
        other_tenant.json.as_array().unwrap().is_empty(),
        "{}",
        other_tenant.text
    );

    // Readiness says whether anybody would be told, and when a test last
    // reached each contact (§8.4).
    let readiness = app
        .admin(Method::GET, "/api/v1/emergency/readiness", None)
        .await;
    let notifications = &readiness.json["notifications"];
    assert_eq!(notifications["deliverable"], 3, "{}", readiness.text);
    assert_eq!(notifications["contacts"][0]["kind"], "email");
    assert!(
        !notifications["contacts"][0]["last_delivery_at"].is_null(),
        "the delivered attempt is remembered: {}",
        readiness.text
    );
    assert!(
        notifications["contacts"][0]["last_test_at"].is_null(),
        "no test call has been made: {}",
        readiness.text
    );
    assert!(
        notifications["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i.as_str().unwrap_or_default().contains("933")),
        "{}",
        readiness.text
    );

    // A tenant with nobody to tell is told so.
    let nobody = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            "/api/v1/emergency/readiness",
            None,
        )
        .await;
    assert_eq!(nobody.json["notifications"]["deliverable"], 0);
    assert!(
        nobody.json["notifications"]["issues"][0]
            .as_str()
            .unwrap()
            .contains("nobody would be told"),
        "{}",
        nobody.text
    );

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
    assert_eq!(defaults.json["cdr_archive_required"], false);
    assert!(
        defaults.json["cdr_archived_before"].is_null(),
        "nothing archived: {}",
        defaults.text
    );

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
    assert_eq!(saved.json["recording_consent_on_failure"], "record");
    assert!(saved.json["recording_notice_interval_secs"].is_null());

    // What a failed announcement does, and the continuing notice.
    for bad in [
        json!({ "recording_consent_on_failure": "maybe" }),
        json!({ "recording_notice_interval_secs": 2 }),
    ] {
        let refused = app
            .admin(Method::PUT, "/api/v1/compliance/policy", Some(bad.clone()))
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{bad}: {}",
            refused.text
        );
    }
    let strict = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({
                "recording_consent_on_failure": "skip",
                "recording_notice_interval_secs": 15
            })),
        )
        .await;
    assert_eq!(strict.status, StatusCode::OK, "{}", strict.text);
    assert_eq!(strict.json["recording_consent_on_failure"], "skip");
    assert_eq!(strict.json["recording_notice_interval_secs"], 15);
    assert_eq!(strict.json["recording_consent"], "announce", "untouched");
    let no_notice = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "recording_notice_interval_secs": null })),
        )
        .await;
    assert!(no_notice.json["recording_notice_interval_secs"].is_null());
    assert_eq!(no_notice.json["recording_consent_on_failure"], "skip");

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
    let required = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "cdr_archive_required": true })),
        )
        .await;
    assert_eq!(required.status, StatusCode::OK, "{}", required.text);
    assert_eq!(required.json["cdr_archive_required"], true);
    assert_eq!(required.json["cdr_retention_days"], 30, "the rest kept");
    // Nothing is archived, so the retention run keeps the records and
    // says why.
    let run = app
        .admin(Method::POST, "/api/v1/compliance/retention/run", None)
        .await;
    assert_eq!(run.status, StatusCode::OK, "{}", run.text);
    assert!(
        run.json["errors"].as_array().unwrap().iter().any(|e| e
            .as_str()
            .unwrap()
            .contains("until the archive confirms them")),
        "{}",
        run.text
    );
    let relaxed = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "cdr_archive_required": false })),
        )
        .await;
    assert_eq!(
        relaxed.json["cdr_archive_required"], false,
        "{}",
        relaxed.text
    );
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
    // Recorded under the name tried, though nobody was signed in
    // (docs/IDENTITY_AND_DEVICES.md phase 1).
    assert_eq!(failed_login["actor"], "ghost", "{failed_login}");
    assert_eq!(failed_login["actor_kind"], "sign_in", "{failed_login}");

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
    // Compliance takes its own permissions: a read-only session can
    // neither read the audit log nor change what is deleted.
    app.create_user_with_role("watcher", "read_only").await;
    let watcher = app.login("watcher", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    for (method, path, body) in [
        (Method::GET, "/api/v1/audit-log", None),
        (Method::GET, "/api/v1/compliance/policy", None),
        (
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "cdr_retention_days": 1 })),
        ),
        (Method::POST, "/api/v1/compliance/retention/run", None),
    ] {
        let refused = app.send_as(&watcher, method.clone(), path, body).await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            refused.text
        );
    }
    let allowed = app
        .send_as(&jwt, Method::GET, "/api/v1/compliance/policy", None)
        .await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.text);
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
        .seed_cdr_full(
            app.tenant_a,
            Some(5_000),
            Some(rust_decimal_macros::dec!(0.10)),
        )
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

    // ── Reading at volume (docs/CDR_PIPELINE.md §9) ──
    // The export as JSON Lines: one record a line.
    let jsonl = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/cdrs/export?format=jsonl",
            None,
        )
        .await;
    assert_eq!(jsonl.status, StatusCode::OK, "{}", jsonl.text);
    assert!(jsonl
        .header("content-type")
        .is_some_and(|ct| ct.starts_with("application/x-ndjson")));
    let exported_records: Vec<serde_json::Value> = jsonl
        .text
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(exported_records.len(), 2, "{}", jsonl.text);
    assert!(exported_records.iter().all(|r| r["kind"] == "call"));
    let bad_format = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/cdrs/export?format=xml",
            None,
        )
        .await;
    assert_eq!(bad_format.status, StatusCode::BAD_REQUEST);

    // A keyset walk: pages of one, each naming the next, until the last.
    let mut walked = Vec::new();
    let mut path = "/api/v1/cdrs?limit=1".to_string();
    loop {
        let page = app.admin_in(app.tenant_a, Method::GET, &path, None).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.text);
        assert_eq!(page.body["total_exact"], true, "{}", page.text);
        for r in page.json.as_array().unwrap() {
            walked.push(r["cdr_id"].as_str().unwrap().to_string());
        }
        match page.body["next_cursor"].as_str() {
            Some(cursor) => path = format!("/api/v1/cdrs?limit=1&cursor={cursor}"),
            None => break,
        }
        assert!(walked.len() <= 50, "the walk ends");
    }
    let total = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/cdrs?limit=1000", None)
        .await;
    let listed = total.json.as_array().unwrap().len();
    // The last page is full when the count divides evenly: one empty page more.
    assert_eq!(walked.len(), listed, "{walked:?}");
    let mut unique = walked.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), walked.len(), "no record twice");
    let deep = app
        .admin_in(app.tenant_a, Method::GET, "/api/v1/cdrs?offset=10001", None)
        .await;
    assert_eq!(deep.status, StatusCode::BAD_REQUEST, "{}", deep.text);
    let bad_cursor = app
        .admin_in(
            app.tenant_a,
            Method::GET,
            "/api/v1/cdrs?cursor=nonsense",
            None,
        )
        .await;
    assert_eq!(
        bad_cursor.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_cursor.text
    );

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
    // The two runs above, and the one the archive requirement held back.
    assert_eq!(runs.json.as_array().unwrap().len(), 3, "{}", runs.text);
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
    let rows = list.json.as_array().unwrap();
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
    let rows = only_running.json.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], running.id.to_string());
    let by_room = app
        .admin(Method::GET, "/api/v1/recordings?room_number=8001", None)
        .await;
    assert_eq!(by_room.json.as_array().unwrap().len(), 1);

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
    let rooms: Vec<&str> = left
        .json
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
    // In the API's error shape, with what the conference server said.
    assert_eq!(res.json["code"], "not_found", "{}", res.text);
    assert_eq!(res.json["details"], "Conference room not active");

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

#[tokio::test]
async fn a_subject_export_is_filed_run_downloaded_and_audited() {
    use fcp_core::subject_request::{run_subject_requests, ExportSettings, LocalMedia};

    let app = require_app!();
    app.create_user("dana", None).await;

    // Validation: a kind that exists now, a subject, a user the tenant has.
    for bad in [
        json!({ "kind": "tenant_deletion", "subjects": [{ "type": "user", "username": "dana" }] }),
        json!({ "kind": "copy", "subjects": [{ "type": "user", "username": "dana" }] }),
        json!({ "kind": "export", "subjects": [] }),
        json!({ "kind": "export", "subjects": [{ "type": "user", "username": "nobody" }] }),
        json!({ "kind": "export", "subjects": [{ "type": "number", "number": "+15550001" }], "reference": "a\r\nb" }),
    ] {
        let refused = app
            .admin(
                Method::POST,
                "/api/v1/compliance/subject-requests",
                Some(bad.clone()),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{bad}: {}",
            refused.text
        );
    }

    // The tenant answers within its own window.
    let policy = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "subject_request_days": 10 })),
        )
        .await;
    assert_eq!(policy.status, StatusCode::OK, "{}", policy.text);
    assert_eq!(policy.json["subject_request_days"], 10);

    let filed = app
        .admin(
            Method::POST,
            "/api/v1/compliance/subject-requests",
            Some(json!({
                "kind": "export",
                "subjects": [{ "type": "user", "username": "dana" }],
                "reference": "DSAR-2026-17"
            })),
        )
        .await;
    assert_eq!(filed.status, StatusCode::CREATED, "{}", filed.text);
    assert_eq!(filed.json["status"], "received");
    let due = rfc3339_secs(&filed.json["due_at"]) - rfc3339_secs(&filed.json["received_at"]);
    assert_eq!(due, 10 * 86_400);
    let id = filed.json["id"].as_str().unwrap().to_string();

    let list = app
        .admin(
            Method::GET,
            "/api/v1/compliance/subject-requests?status=received",
            None,
        )
        .await;
    assert_eq!(list.body["total"], 1, "{}", list.text);
    let other = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}"),
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    let early = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}/bundle"),
            None,
        )
        .await;
    assert_eq!(early.status, StatusCode::CONFLICT, "{}", early.text);

    // The worker runs it.
    let exports = tempfile::tempdir().unwrap();
    let settings = ExportSettings {
        exports_dir: exports.path().to_path_buf(),
        bundle_ttl: Duration::from_secs(86_400),
    };
    let media = LocalMedia::default();
    let access = fcp_core::subject_request::WorkerAccess {
        media: &media,
        remover: &media,
        dead_letters: None,
    };
    let finished = run_subject_requests(&app.dyn_storage, access, &settings)
        .await
        .unwrap();
    assert_eq!(finished.len(), 1);

    let done = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}"),
            None,
        )
        .await;
    assert_eq!(done.json["status"], "completed", "{}", done.text);
    assert_eq!(
        done.json["report"]["classes"]["profile"]["items"], 1,
        "{}",
        done.text
    );
    assert!(done.json["expansion"]["identifiers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i == "dana"));
    let sha = done.json["bundle"]["sha256"].as_str().unwrap().to_string();

    let bundle = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}/bundle"),
            None,
        )
        .await;
    assert_eq!(bundle.status, StatusCode::OK, "{}", bundle.text);
    assert_eq!(bundle.headers["content-type"], "application/gzip");
    assert_eq!(bundle.headers["x-content-sha256"], sha.as_str());
    assert_eq!(&bundle.bytes[..2], &[0x1f, 0x8b], "gzip");
    let audited = wait_for_audit(&app, "/api/v1/audit-log?limit=50", |e| {
        e["action"] == "GET /api/v1/compliance/subject-requests/:id/bundle" && e["status"] == 200
    })
    .await;
    assert_eq!(audited["actor"], "static_token", "{audited}");

    // A read-only session may do none of it.
    app.create_user_with_role("onlooker", "read_only").await;
    let onlooker = app.login("onlooker", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    for (method, path) in [
        (
            Method::GET,
            "/api/v1/compliance/subject-requests".to_string(),
        ),
        (
            Method::GET,
            format!("/api/v1/compliance/subject-requests/{id}/bundle"),
        ),
    ] {
        let refused = app.send_as(&onlooker, method.clone(), &path, None).await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            refused.text
        );
    }

    // Once the bundle is removed it is gone.
    assert_eq!(
        fcp_core::subject_request::remove_expired_bundles(
            &app.dyn_storage,
            SystemTime::now() + Duration::from_secs(2 * 86_400)
        )
        .await
        .unwrap(),
        1
    );
    let gone = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}/bundle"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::GONE, "{}", gone.text);
    app.finish().await;
}

/// Run the subject request worker once, with this host's files only.
async fn run_worker(app: &App) -> Vec<fcp_core::subject_request::SubjectRequest> {
    let media = fcp_core::subject_request::LocalMedia::default();
    let exports = tempfile::tempdir().unwrap();
    fcp_core::subject_request::run_subject_requests(
        &app.dyn_storage,
        fcp_core::subject_request::WorkerAccess {
            media: &media,
            remover: &media,
            dead_letters: None,
        },
        &fcp_core::subject_request::ExportSettings {
            exports_dir: exports.path().to_path_buf(),
            bundle_ttl: Duration::from_secs(60),
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn an_erasure_is_previewed_confirmed_by_a_person_and_logged_under_its_request() {
    let app = require_app!();
    app.create_user("erin", None).await;
    app.create_user("frank", None).await;

    let policy = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "cdr_erasure": "sometimes" })),
        )
        .await;
    assert_eq!(policy.status, StatusCode::BAD_REQUEST, "{}", policy.text);
    let policy = app
        .admin(
            Method::PUT,
            "/api/v1/compliance/policy",
            Some(json!({ "cdr_erasure": "delete" })),
        )
        .await;
    assert_eq!(policy.json["cdr_erasure"], "delete", "{}", policy.text);

    let filed = app
        .admin(
            Method::POST,
            "/api/v1/compliance/subject-requests",
            Some(
                json!({ "kind": "erasure", "subjects": [{ "type": "user", "username": "erin" }] }),
            ),
        )
        .await;
    assert_eq!(filed.status, StatusCode::CREATED, "{}", filed.text);
    let id = filed.json["id"].as_str().unwrap().to_string();
    let confirm = format!("/api/v1/compliance/subject-requests/{id}/confirm");
    let early = app.admin(Method::POST, &confirm, None).await;
    assert_eq!(
        early.status,
        StatusCode::CONFLICT,
        "nothing to confirm yet: {}",
        early.text
    );

    // The preview, and nothing gone.
    run_worker(&app).await;
    let previewed = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}"),
            None,
        )
        .await;
    assert_eq!(previewed.json["status"], "preview", "{}", previewed.text);
    assert_eq!(
        previewed.json["preview"]["classes"]["profile"]["items"], 1,
        "{}",
        previewed.text
    );
    assert!(previewed.json["erasure"].is_null());
    let erin = app.admin(Method::GET, "/api/v1/users/erin", None).await;
    assert_eq!(erin.status, StatusCode::OK, "still there after the preview");

    let confirmed = app.admin(Method::POST, &confirm, None).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text);
    assert_eq!(confirmed.json["status"], "confirmed");
    assert_eq!(confirmed.json["confirmed_by"], "static_token");
    let twice = app.admin(Method::POST, &confirm, None).await;
    assert_eq!(twice.status, StatusCode::CONFLICT);

    run_worker(&app).await;
    let done = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/subject-requests/{id}"),
            None,
        )
        .await;
    assert_eq!(done.json["status"], "completed", "{}", done.text);
    assert_eq!(
        done.json["erasure"]["classes"]["profile"]["items"], 1,
        "{}",
        done.text
    );
    let erin = app.admin(Method::GET, "/api/v1/users/erin", None).await;
    assert_eq!(erin.status, StatusCode::NOT_FOUND);
    let frank = app.admin(Method::GET, "/api/v1/users/frank", None).await;
    assert_eq!(frank.status, StatusCode::OK, "only the subject");
    let logged = app
        .admin(
            Method::GET,
            &format!("/api/v1/compliance/deletions?request_id={id}"),
            None,
        )
        .await;
    let entries = logged.json.as_array().unwrap();
    assert!(!entries.is_empty(), "{}", logged.text);
    assert!(
        entries
            .iter()
            .all(|e| e["cause"] == "erasure" && e["request_id"] == id.as_str()),
        "{}",
        logged.text
    );
    app.finish().await;
}

#[tokio::test]
async fn deleting_a_tenant_is_a_request_refused_under_a_hold() {
    let app = require_app!();
    let b = app.tenant_b.to_string();
    let path = format!("/api/v1/tenants/{b}");

    // Under a hold: refused, and the tenant untouched.
    let hold = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/compliance/holds",
            Some(json!({ "matter": "Audit 3" })),
        )
        .await;
    assert_eq!(hold.status, StatusCode::CREATED, "{}", hold.text);
    let refused = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    let hold_id = hold.json["id"].as_str().unwrap();
    let released = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            &format!("/api/v1/compliance/holds/{hold_id}/release"),
            Some(json!({ "reason": "closed" })),
        )
        .await;
    assert_eq!(released.status, StatusCode::OK, "{}", released.text);

    let accepted = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.text);
    assert_eq!(accepted.json["kind"], "tenant_deletion");
    assert_eq!(accepted.json["status"], "confirmed");
    let request = accepted.json["id"].as_str().unwrap().to_string();
    let again = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(again.json["id"], request.as_str(), "one request, not two");
    let closed = app.admin(Method::GET, &path, None).await;
    assert_eq!(closed.json["status"], "closed", "{}", closed.text);

    run_worker(&app).await;
    let gone = app.admin(Method::GET, &path, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);
    let done = app
        .admin(Method::GET, &format!("{path}/deletion"), None)
        .await;
    assert_eq!(done.json["id"], request.as_str(), "{}", done.text);
    assert_eq!(done.json["status"], "completed", "{}", done.text);
    assert_eq!(
        done.json["erasure"]["tables"]["tenants"], 1,
        "{}",
        done.text
    );
    app.finish().await;
}

/// A tenant's routing rules decide its calls on every call manager
/// (`docs/TENANT_TRUNKS.md` §12, phase 6b), so managing them takes the
/// trunk permissions, a rule is checked as a node will compile it, a trunk
/// it names must be the tenant's own, and every change is announced.
#[tokio::test]
async fn routing_rules_are_checked_owned_and_announced() {
    // Carrier mode, where a tenant admin manages the tenant's trunks.
    let app = require_app!(DeploymentMode::Carrier);

    let ours = app
        .admin(
            Method::POST,
            "/api/v1/trunks",
            Some(json!({ "name": "intl", "hosts": ["sip.carrier.example.net"] })),
        )
        .await;
    assert_eq!(ours.status, StatusCode::CREATED, "{}", ours.text);
    let ours = ours.json["id"].as_str().unwrap().to_string();
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/trunks",
            Some(json!({ "name": "intl", "hosts": ["sip.other.example.net"] })),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.text);
    let theirs = theirs.json["id"].as_str().unwrap().to_string();

    let rule = |extra: Value| {
        let mut body = json!({ "name": "international", "priority": 10, "enabled": true });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };
    let refused = |text: &str, needle: &str| {
        assert!(text.contains(needle), "expected '{needle}' in: {text}");
    };

    // What a node would refuse to compile is refused here.
    for (body, needle) in [
        (
            rule(json!({ "action": "allow", "destination_pattern": "([" })),
            "destination",
        ),
        (
            rule(json!({ "action": "allow", "time_window_days": ["Someday"] })),
            "Someday",
        ),
        (
            rule(json!({ "action": "allow", "source_ip_ranges": ["10.0.0.0/33"] })),
            "10.0.0.0/33",
        ),
        (
            rule(json!({ "action": "route_to_trunk" })),
            "must name a trunk",
        ),
        // Another tenant's trunk, however it is named.
        (
            rule(json!({ "action": "route_to_trunk", "target_trunk_id": theirs })),
            "not one of this tenant's trunks",
        ),
    ] {
        let res = app
            .admin(Method::POST, "/api/v1/routing/rules", Some(body))
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
        refused(&res.text, needle);
    }

    // A good one is stored and announced, so nodes re-read the rules now.
    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .expect("subscribe");
    let created = app
        .admin(
            Method::POST,
            "/api/v1/routing/rules",
            Some(rule(json!({
                "action": "route_to_trunk",
                "destination_pattern": "^\\+44",
                "target_trunk_id": ours,
                "time_window_days": ["Monday", "Friday"],
                "time_window_start": "09:00",
                "time_window_end": "17:00",
                "time_window_timezone": "Europe/London",
            }))),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let announced =
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.next().await {
                    Some(PlatformEvent::System(SystemEvent::ConfigChanged {
                        entity_type, ..
                    })) if entity_type == "routing_rule" => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await;
    assert_eq!(announced, Ok(true), "the change is announced");

    // An end user may neither read nor write the rules.
    app.create_user_with_role("phone-user", "end_user").await;
    let jwt = app.login("phone-user", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = app
        .send_as(&jwt, Method::GET, "/api/v1/routing/rules", None)
        .await;
    assert_eq!(listed.status, StatusCode::FORBIDDEN, "{}", listed.text);
    let written = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/routing/rules",
            Some(rule(
                json!({ "action": "deny", "destination_pattern": ".*" }),
            )),
        )
        .await;
    assert_eq!(written.status, StatusCode::FORBIDDEN, "{}", written.text);

    app.finish().await;
}

/// A tenant's least-cost routes (`docs/TENANT_TRUNKS.md` §12, phase 6c):
/// checked, on the tenant's own trunks only, one per prefix and trunk,
/// announced when they change, gone with their trunk, and closed to an end
/// user.
#[tokio::test]
async fn least_cost_routes_are_checked_owned_and_go_with_their_trunk() {
    let app = require_app!(DeploymentMode::Carrier);

    let trunk = |name: &str| json!({ "name": name, "hosts": ["sip.carrier.example.net"] });
    let ours = app
        .admin(Method::POST, "/api/v1/trunks", Some(trunk("cheap-uk")))
        .await;
    assert_eq!(ours.status, StatusCode::CREATED, "{}", ours.text);
    let ours = ours.json["id"].as_str().unwrap().to_string();
    let theirs = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/trunks",
            Some(trunk("cheap-uk")),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::CREATED, "{}", theirs.text);
    let theirs = theirs.json["id"].as_str().unwrap().to_string();

    for (body, needle) in [
        (
            json!({ "prefix": "+44x", "trunk_id": ours, "cost_per_minute": 0.01 }),
            "only digits",
        ),
        (
            json!({ "prefix": "+44", "trunk_id": ours, "cost_per_minute": -0.01 }),
            "negative",
        ),
        (
            json!({ "prefix": "+44", "trunk_id": theirs, "cost_per_minute": 0.01 }),
            "not one of this tenant's trunks",
        ),
    ] {
        let res = app
            .admin(Method::POST, "/api/v1/lcr-routes", Some(body))
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.text);
        assert!(
            res.text.contains(needle),
            "expected '{needle}' in {}",
            res.text
        );
    }

    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .expect("subscribe");
    let created = app
        .admin(
            Method::POST,
            "/api/v1/lcr-routes",
            Some(json!({
                "prefix": "+44",
                "trunk_id": ours,
                "cost_per_minute": 0.012,
                "description": "UK via the cheap carrier",
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    assert_eq!(created.json["priority"], 100, "{}", created.text);
    assert_eq!(created.json["cost_per_minute"], 0.012, "{}", created.text);
    let id = created.json["id"].as_str().unwrap().to_string();
    let announced =
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.next().await {
                    Some(PlatformEvent::System(SystemEvent::ConfigChanged {
                        entity_type, ..
                    })) if entity_type == "lcr_route" => return true,
                    Some(_) => continue,
                    None => return false,
                }
            }
        })
        .await;
    assert_eq!(announced, Ok(true), "the change is announced");

    // A second route for the same prefix on the same trunk is a conflict.
    let again = app
        .admin(
            Method::POST,
            "/api/v1/lcr-routes",
            Some(json!({ "prefix": "+44", "trunk_id": ours, "cost_per_minute": 0.02 })),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);

    let patched = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/lcr-routes/{id}"),
            Some(json!({ "cost_per_minute": 0.009, "priority": 5 })),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text);
    assert_eq!(patched.json["cost_per_minute"], 0.009, "{}", patched.text);
    let listed = app
        .admin(
            Method::GET,
            &format!("/api/v1/lcr-routes?trunk_id={ours}"),
            None,
        )
        .await;
    assert_eq!(
        listed.json.as_array().map(Vec::len),
        Some(1),
        "{}",
        listed.text
    );

    // Another tenant cannot see it.
    let hidden = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/lcr-routes/{id}"),
            None,
        )
        .await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND, "{}", hidden.text);

    // An end user may not read or write them.
    app.create_user_with_role("phone-user", "end_user").await;
    let jwt = app.login("phone-user", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let denied = app
        .send_as(&jwt, Method::GET, "/api/v1/lcr-routes", None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text);

    // A route goes with its trunk.
    let gone = app
        .admin(Method::DELETE, &format!("/api/v1/trunks/{ours}"), None)
        .await;
    assert!(gone.status.is_success(), "{}", gone.text);
    let listed = app.admin(Method::GET, "/api/v1/lcr-routes", None).await;
    assert_eq!(listed.json, json!([]), "{}", listed.text);

    app.finish().await;
}

/// Routing rules belong to the tenant's administrators in every mode
/// (`docs/TENANT_TRUNKS.md` §12, phase 6d): in enterprise mode, where the
/// trunks are the platform's, a tenant admin writes allow and deny rules,
/// but not a rule that routes to a trunk — that names a carrier. The route
/// preview answers 503 when there is no call manager to ask.
#[tokio::test]
async fn enterprise_tenant_admins_manage_rules_but_not_carriers() {
    let app = require_app!(DeploymentMode::Enterprise);
    app.create_user_with_role("ent-admin", "tenant_admin").await;
    let jwt = app.login("ent-admin", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();

    let deny = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/routing/rules",
            Some(json!({
                "name": "no-premium",
                "priority": 10,
                "enabled": true,
                "action": "deny",
                "destination_pattern": "^\\+1900",
            })),
        )
        .await;
    assert_eq!(deny.status, StatusCode::CREATED, "{}", deny.text);
    let listed = app
        .send_as(&jwt, Method::GET, "/api/v1/routing/rules", None)
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);

    let routed = app
        .send_as(
            &jwt,
            Method::POST,
            "/api/v1/routing/rules",
            Some(json!({
                "name": "via-group",
                "priority": 20,
                "enabled": true,
                "action": "route_to_trunk",
                "target_trunk_group": "intl",
            })),
        )
        .await;
    assert_eq!(routed.status, StatusCode::FORBIDDEN, "{}", routed.text);

    // Turning the deny rule into a trunk route is refused the same way.
    let id = deny.json["id"].as_str().unwrap();
    let turned = app
        .send_as(
            &jwt,
            Method::PATCH,
            &format!("/api/v1/routing/rules/{id}"),
            Some(json!({ "action": "route_to_trunk", "target_trunk_group": "intl" })),
        )
        .await;
    assert_eq!(turned.status, StatusCode::FORBIDDEN, "{}", turned.text);

    // No call manager to ask: 503, not a guess.
    let preview = app
        .send_as(
            &jwt,
            Method::GET,
            "/api/v1/routing/preview?to=%2B442079460000",
            None,
        )
        .await;
    assert_eq!(
        preview.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        preview.text
    );

    app.finish().await;
}

/// Sites (`docs/HYBRID_CONTROL_PLANE.md` phase 1): made in the cloud with a
/// one-time token shown once, listed, disabled and enabled, deleted; a
/// disabled site mints no token; another tenant sees nothing.
#[tokio::test]
async fn sites_are_made_with_a_token_shown_once_and_managed_in_their_tenant() {
    let app = require_app!();

    let made = app
        .admin(
            Method::POST,
            "/api/v1/sites",
            Some(json!({ "name": "acme-hq" })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = made.json["id"].as_str().expect("a site id").to_string();
    assert!(id.starts_with("site-"), "{id}");
    assert_eq!(made.json["status"], "pending");
    assert_eq!(made.json["link_up"], false);
    let token = made.json["enrollment_token"]
        .as_str()
        .expect("the token, shown once")
        .to_string();
    assert!(token.len() >= 40, "{token}");
    assert!(rfc3339_secs(&made.json["enrollment_token_expires_at"]) > 0);

    // Listed and read back, without the token.
    let listed = app.admin(Method::GET, "/api/v1/sites", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let list = listed.json.as_array().expect("a list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "acme-hq");
    assert!(list[0].get("enrollment_token").is_none());
    let got = app
        .admin(Method::GET, &format!("/api/v1/sites/{id}"), None)
        .await;
    assert_eq!(got.status, StatusCode::OK, "{}", got.text);
    assert_eq!(got.json["tenant_id"], app.tenant_a.to_string());

    // Another tenant sees nothing of it.
    let other = app
        .send(
            Method::GET,
            &format!("/api/v1/sites/{id}"),
            &[
                (AUTH_HEADER, BEARER),
                ("x-fcp-tenant-id", &app.tenant_b.to_string()),
            ],
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "{}", other.text);

    // A new token for it; then disabled, which mints none; then enabled
    // again — pending, since no link ever enrolled.
    let again = app
        .admin(
            Method::POST,
            &format!("/api/v1/sites/{id}/enrollment-token"),
            None,
        )
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_ne!(again.json["enrollment_token"].as_str().unwrap(), token);
    let disabled = app
        .admin(Method::POST, &format!("/api/v1/sites/{id}/disable"), None)
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.text);
    assert_eq!(disabled.json["status"], "disabled");
    let refused = app
        .admin(
            Method::POST,
            &format!("/api/v1/sites/{id}/enrollment-token"),
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.text);
    let enabled = app
        .admin(Method::POST, &format!("/api/v1/sites/{id}/enable"), None)
        .await;
    assert_eq!(enabled.status, StatusCode::OK, "{}", enabled.text);
    assert_eq!(enabled.json["status"], "pending");

    // A name is required.
    let nameless = app
        .admin(Method::POST, "/api/v1/sites", Some(json!({ "name": "  " })))
        .await;
    assert_eq!(
        nameless.status,
        StatusCode::BAD_REQUEST,
        "{}",
        nameless.text
    );

    // Gone.
    let deleted = app
        .admin(Method::DELETE, &format!("/api/v1/sites/{id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let gone = app
        .admin(Method::GET, &format!("/api/v1/sites/{id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);

    app.finish().await;
}

// ─── Schedules (docs/ENTERPRISE_CALLING.md §3.5) ─────────────────────────

#[tokio::test]
async fn schedules_are_the_tenants_and_say_their_state() {
    let app = require_app!();
    let created = app
        .admin(
            Method::POST,
            "/api/v1/schedules",
            Some(json!({
                "name": "Office",
                "timezone": "America/New_York",
                "weekly": [{"days": ["Mon", "Tue", "Wed", "Thu", "Fri"], "start": "09:00", "end": "17:00"}],
                "holidays": [{"name": "Thanksgiving", "from": "2026-11-26", "to": "2026-11-27"}]
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let id = created.json["id"].as_str().unwrap().to_string();
    assert!(
        ["open", "closed", "holiday"].contains(&created.json["state"].as_str().unwrap()),
        "{}",
        created.text
    );
    assert_eq!(created.json["weekly"][0]["start"], "09:00");

    let listed = app.admin(Method::GET, "/api/v1/schedules", None).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json.as_array().unwrap().len(), 1);

    // Replaced whole; an unknown timezone is refused.
    let updated = app
        .admin(
            Method::PUT,
            &format!("/api/v1/schedules/{id}"),
            Some(json!({ "name": "Always", "timezone": "UTC",
                         "weekly": [{"days": ["Mon","Tue","Wed","Thu","Fri","Sat","Sun"], "start": "00:00", "end": "00:00"}] })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text);
    assert_eq!(updated.json["state"], "open");
    assert!(updated.json["holidays"].as_array().unwrap().is_empty());
    let bad = app
        .admin(
            Method::PUT,
            &format!("/api/v1/schedules/{id}"),
            Some(json!({ "name": "Bad", "timezone": "Mars/Olympus" })),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    // Another tenant neither sees it nor can name it.
    let other = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/schedules/{id}"),
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "{}", other.text);

    let gone = app
        .admin(Method::DELETE, &format!("/api/v1/schedules/{id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    let after = app
        .admin(Method::GET, &format!("/api/v1/schedules/{id}"), None)
        .await;
    assert_eq!(after.status, StatusCode::NOT_FOUND, "{}", after.text);
}

// ─── Class of service (docs/ENTERPRISE_CALLING.md §3.7) ──────────────────

#[tokio::test]
async fn classes_of_service_are_assigned_by_an_admin_and_numbers_read_by_the_tenants_country() {
    let app = require_app!();
    app.create_user_with_role("classy", "end_user").await;

    // The tenant dials as the UK does, keeps London time, and calls 118
    // numbers premium.
    let tenant = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({ "calling": {
                "country": "gb",
                "timezone": "Europe/London",
                "destination_overrides": [{ "prefix": "118", "class": "premium" }]
            }})),
        )
        .await;
    assert_eq!(tenant.status, StatusCode::OK, "{}", tenant.text);
    assert_eq!(tenant.json["calling"]["country"], "GB");
    let bad_country = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({ "calling": { "country": "XX" } })),
        )
        .await;
    assert_eq!(
        bad_country.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_country.text
    );
    for (dialed, class) in [
        ("02079460000", "national"),
        ("08001234567", "toll_free"),
        ("0015185550100", "international"),
        ("118118", "premium"),
        ("2001", "internal"),
        ("999", "emergency"),
    ] {
        let got = app
            .admin(
                Method::GET,
                &format!("/api/v1/classes-of-service/classify?dialed={dialed}"),
                None,
            )
            .await;
        assert_eq!(got.status, StatusCode::OK, "{}", got.text);
        assert_eq!(got.json["class"], class, "{dialed}: {}", got.text);
    }

    // A new schedule without a timezone keeps the tenant's.
    let schedule = app
        .admin(
            Method::POST,
            "/api/v1/schedules",
            Some(json!({ "name": "Office" })),
        )
        .await;
    assert_eq!(schedule.status, StatusCode::CREATED, "{}", schedule.text);
    assert_eq!(schedule.json["timezone"], "Europe/London");

    // Two classes; the second default takes the mark from the first.
    let local = app
        .admin(
            Method::POST,
            "/api/v1/classes-of-service",
            Some(
                json!({ "name": "Local", "allowed": ["internal", "national"], "is_default": true }),
            ),
        )
        .await;
    assert_eq!(local.status, StatusCode::CREATED, "{}", local.text);
    assert_eq!(local.json["max_calls"], 2, "two unless said");
    assert!(
        local.json["allowed"]
            .as_array()
            .unwrap()
            .contains(&json!("emergency")),
        "emergency is always allowed: {}",
        local.text
    );
    let local_id = local.json["id"].as_str().unwrap().to_string();
    let lobby = app
        .admin(
            Method::POST,
            "/api/v1/classes-of-service",
            Some(
                json!({ "name": "Lobby", "allowed": ["internal"], "max_calls": null,
                         "caller_id": "withheld", "is_default": true }),
            ),
        )
        .await;
    assert_eq!(lobby.status, StatusCode::CREATED, "{}", lobby.text);
    assert!(lobby.json["max_calls"].is_null(), "{}", lobby.text);
    let lobby_id = lobby.json["id"].as_str().unwrap().to_string();
    let listed = app
        .admin(Method::GET, "/api/v1/classes-of-service", None)
        .await;
    let defaults: Vec<_> = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["is_default"] == true)
        .map(|c| c["name"].clone())
        .collect();
    assert_eq!(defaults, vec![json!("Lobby")], "{}", listed.text);
    let dup = app
        .admin(
            Method::POST,
            "/api/v1/classes-of-service",
            Some(json!({ "name": "local" })),
        )
        .await;
    assert_eq!(dup.status, StatusCode::CONFLICT, "{}", dup.text);

    // An admin gives the user a class; the user cannot change it.
    let assigned = app
        .admin(
            Method::PUT,
            "/api/v1/users/classy/calling",
            Some(json!({ "class_of_service_id": local_id })),
        )
        .await;
    assert_eq!(assigned.status, StatusCode::OK, "{}", assigned.text);
    assert_eq!(assigned.json["class_of_service_id"], local_id.as_str());
    let jwt = app.login("classy", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let own = app
        .send_as(
            &jwt,
            Method::PUT,
            "/api/v1/me/calling",
            Some(json!({ "class_of_service_id": lobby_id })),
        )
        .await;
    assert_eq!(own.status, StatusCode::FORBIDDEN, "{}", own.text);
    let other_tenants = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/classes-of-service/{local_id}"),
            None,
        )
        .await;
    assert_eq!(
        other_tenants.status,
        StatusCode::NOT_FOUND,
        "{}",
        other_tenants.text
    );

    // A class a user has is not deleted from under them.
    let in_use = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/classes-of-service/{local_id}"),
            None,
        )
        .await;
    assert_eq!(in_use.status, StatusCode::CONFLICT, "{}", in_use.text);
    let cleared = app
        .admin(
            Method::PUT,
            "/api/v1/users/classy/calling",
            Some(json!({ "class_of_service_id": "" })),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.text);
    assert!(
        cleared.json["class_of_service_id"].is_null(),
        "{}",
        cleared.text
    );
    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/classes-of-service/{local_id}"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);

    app.finish().await;
}

#[tokio::test]
async fn hoteling_hosts_credentials_and_sessions() {
    use fcp_core::HotelingStore;
    let app = require_app!();
    let owner_id = app.create_user_with_role("deskowner", "end_user").await;
    let guest_id = app.create_user_with_role("guest", "end_user").await;

    // A host with credentials of its own (docs/ENTERPRISE_CALLING.md §3.8):
    // a username no user has, and a password long enough.
    let taken = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({ "name": "Lobby", "device_type": "desk_phone",
                         "sip_username": "guest", "sip_password": "long-enough-pw" })),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);
    let short = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({ "name": "Lobby", "device_type": "desk_phone",
                         "sip_username": "lobby-1", "sip_password": "short" })),
        )
        .await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST, "{}", short.text);
    let host = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({ "name": "Lobby", "device_type": "desk_phone",
                         "sip_uri": "sip:lobby-1@example.com",
                         "sip_username": "lobby-1", "sip_password": "long-enough-pw",
                         "hoteling_host": true })),
        )
        .await;
    assert_eq!(host.status, StatusCode::CREATED, "{}", host.text);
    assert_eq!(host.json["sip_username"], "lobby-1");
    assert_eq!(host.json["has_sip_credentials"], true);
    assert_eq!(host.json["hoteling_host"], true);
    for secret in ["sip_ha1_hash", "sip_provisioning_password", "sip_password"] {
        assert!(
            host.json.get(secret).is_none(),
            "{secret} returned: {}",
            host.text
        );
    }
    let host_id = host.json["id"].as_str().unwrap().to_string();
    let path = format!("/api/v1/devices/{host_id}");

    // A new username needs a new password; the owner can be set and cleared.
    let renamed = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "sip_username": "lobby-2" })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::BAD_REQUEST, "{}", renamed.text);
    let owned = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "owner_user_id": owner_id })),
        )
        .await;
    assert_eq!(owned.status, StatusCode::OK, "{}", owned.text);
    assert_eq!(owned.json["owner_user_id"], owner_id.as_str());
    let unowned = app
        .admin(Method::PATCH, &path, Some(json!({ "owner_user_id": null })))
        .await;
    assert_eq!(unowned.status, StatusCode::OK, "{}", unowned.text);
    assert!(unowned.json["owner_user_id"].is_null(), "{}", unowned.text);

    // The tenant's session length.
    let bad_hours = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({ "calling": { "hoteling_session_hours": 0 } })),
        )
        .await;
    assert_eq!(
        bad_hours.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_hours.text
    );

    // A guest logged in (a login happens at the phone; here, in the store).
    let host_device: fcp_core::DeviceId = host_id.parse().unwrap();
    let guest: fcp_core::UserId = guest_id.parse().unwrap();
    let start = || {
        fcp_core::HotelingSession::start(
            app.tenant_a,
            host_device,
            guest,
            Duration::from_secs(3600),
        )
    };
    app.storage
        .start_hoteling_session(start())
        .await
        .expect("session");
    let list = app
        .admin(Method::GET, "/api/v1/hoteling/sessions", None)
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    assert_eq!(list.json.as_array().unwrap().len(), 1, "{}", list.text);
    assert_eq!(list.json[0]["username"], "guest");
    assert_eq!(list.json[0]["device_name"], "Lobby");
    assert_eq!(list.json[0]["active"], true);

    // The guest sees it and logs out.
    let jwt = app.login("guest", PASSWORD).await.json["token"]
        .as_str()
        .expect("jwt")
        .to_string();
    let mine = app
        .send_as(&jwt, Method::GET, "/api/v1/me/hoteling", None)
        .await;
    assert_eq!(mine.status, StatusCode::OK, "{}", mine.text);
    assert_eq!(mine.json["device_id"], host_id.as_str());
    let out = app
        .send_as(&jwt, Method::DELETE, "/api/v1/me/hoteling", None)
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT, "{}", out.text);
    let mine = app
        .send_as(&jwt, Method::GET, "/api/v1/me/hoteling", None)
        .await;
    assert!(mine.json.is_null(), "{}", mine.text);
    let again = app
        .send_as(&jwt, Method::DELETE, "/api/v1/me/hoteling", None)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "{}", again.text);

    // An admin ends one; a host that stops being a host ends another.
    let second = start();
    app.storage
        .start_hoteling_session(second.clone())
        .await
        .expect("session");
    let ended = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/hoteling/sessions/{}", second.id),
            None,
        )
        .await;
    assert_eq!(ended.status, StatusCode::NO_CONTENT, "{}", ended.text);
    let third = start();
    app.storage
        .start_hoteling_session(third.clone())
        .await
        .expect("session");
    let no_host = app
        .admin(
            Method::PATCH,
            &path,
            Some(json!({ "hoteling_host": false })),
        )
        .await;
    assert_eq!(no_host.status, StatusCode::OK, "{}", no_host.text);
    let active = app
        .admin(Method::GET, "/api/v1/hoteling/sessions", None)
        .await;
    assert_eq!(active.json.as_array().unwrap().len(), 0, "{}", active.text);
    let all = app
        .admin(Method::GET, "/api/v1/hoteling/sessions?all=true", None)
        .await;
    // Started within one second, the three tie on their start: what each
    // ended by, not their order.
    let mut ends: Vec<&str> = all
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["ended_by"].as_str().unwrap())
        .collect();
    ends.sort();
    assert_eq!(ends, ["admin", "admin", "logout"], "{}", all.text);

    // A starting session replaces the guest's other one.
    app.storage
        .start_hoteling_session(start())
        .await
        .expect("session");
    let replaced = app
        .storage
        .start_hoteling_session(start())
        .await
        .expect("session");
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].ended_by, Some(fcp_core::SessionEnd::Replaced));

    app.finish().await;
}

// ─── Webhooks (docs/DEVELOPER_SURFACE.md §3, phase 5) ───────────────────

/// What a test's webhook receiver saw: each request's signature header,
/// event header and body.
#[derive(Clone, Default)]
struct Receiver {
    seen: Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    answer: Arc<std::sync::atomic::AtomicU16>,
}

impl Receiver {
    /// Listen on loopback; returns the receiver and its URL.
    async fn start() -> (Self, String) {
        let receiver = Receiver::default();
        receiver
            .answer
            .store(200, std::sync::atomic::Ordering::SeqCst);
        let r = receiver.clone();
        let app = Router::new().route(
            "/hook",
            axum::routing::post(move |headers: HeaderMap, body: String| {
                let r = r.clone();
                async move {
                    let h = |n: &str| {
                        headers
                            .get(n)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string()
                    };
                    r.seen
                        .lock()
                        .unwrap()
                        .push((h("x-fcp-signature"), h("x-fcp-event"), body));
                    StatusCode::from_u16(r.answer.load(std::sync::atomic::Ordering::SeqCst))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        (receiver, url)
    }

    fn last(&self) -> (String, String, String) {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a delivery")
    }

    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

/// `t` and the `v1` values of an `X-FCP-Signature`.
fn signature_parts(header: &str) -> (u64, Vec<String>) {
    let mut t = 0;
    let mut v1 = Vec::new();
    for piece in header.split(',') {
        match piece.split_once('=') {
            Some(("t", value)) => t = value.parse().unwrap(),
            Some(("v1", value)) => v1.push(value.to_string()),
            _ => {}
        }
    }
    (t, v1)
}

/// A receiver's check: some `v1` is the HMAC of `"<t>.<body>"` under
/// `secret`, and `t` is within five minutes of `now`.
fn verifies(secret: &str, header: &str, body: &str, now: u64) -> bool {
    let (t, v1) = signature_parts(header);
    now.abs_diff(t) <= 300 && v1.contains(&fcp_webhooks::sign(secret, t, body))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A webhook is the tenant's: made with a server secret shown once,
/// refused a private or non-HTTPS target or an unknown event, invisible to
/// another tenant, rotated with an overlap, and closed to a role without
/// `webhook_manage`.
#[tokio::test]
async fn webhooks_are_the_tenants_and_their_secret_is_shown_once() {
    let app = require_app!();

    for (url, why) in [
        ("https://127.0.0.1/hook", "loopback"),
        ("https://169.254.169.254/latest/meta-data", "metadata"),
        ("https://10.0.0.8/hook", "private"),
        ("https://[::1]/hook", "loopback v6"),
        ("https://localhost/hook", "localhost"),
        ("http://93.184.216.34/hook", "plain HTTP"),
        ("https://user:pw@93.184.216.34/hook", "credentials"),
    ] {
        let refused = app
            .admin(
                Method::POST,
                "/api/v1/webhooks",
                Some(json!({ "url": url })),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{why}: {}",
            refused.text
        );
        assert_eq!(refused.json["code"], "bad_request", "{}", refused.text);
    }
    let unknown = app
        .admin(
            Method::POST,
            "/api/v1/webhooks",
            Some(json!({ "url": "https://93.184.216.34/hook", "events": ["call.nonsense"] })),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.text);

    let made = app
        .admin(
            Method::POST,
            "/api/v1/webhooks",
            Some(json!({
                "url": "https://93.184.216.34/hook",
                "events": ["call.answered", "voicemail.*"],
                "description": "CRM",
            })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let secret = made.json["secret"].as_str().unwrap().to_string();
    assert!(
        secret.starts_with("whsec_") && secret.len() > 40,
        "{secret}"
    );
    let id = made.json["webhook"]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("whk-"), "{id}");
    assert_eq!(made.json["webhook"]["tenant_id"], app.tenant_a.to_string());

    let read = app
        .admin(Method::GET, &format!("/api/v1/webhooks/{id}"), None)
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", read.text);
    assert!(
        read.json.get("secret").is_none(),
        "the secret is shown once: {}",
        read.text
    );
    assert!(!read.text.contains(&secret), "{}", read.text);
    assert!(secret.starts_with(read.json["secret_prefix"].as_str().unwrap()));
    assert_eq!(read.json["events"], json!(["call.answered", "voicemail.*"]));

    // Another tenant neither lists nor reads it.
    let listed = app.admin(Method::GET, "/api/v1/webhooks", None).await;
    assert_eq!(listed.json.as_array().unwrap().len(), 1, "{}", listed.text);
    let other = app
        .admin_in(app.tenant_b, Method::GET, "/api/v1/webhooks", None)
        .await;
    assert_eq!(other.json, json!([]), "{}", other.text);
    let other = app
        .admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/webhooks/{id}"),
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "{}", other.text);

    // A change is checked like a creation.
    let patched = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/webhooks/{id}"),
            Some(json!({ "events": ["call.*"], "description": "" })),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text);
    assert_eq!(patched.json["events"], json!(["call.*"]));
    assert!(patched.json["description"].is_null(), "{}", patched.text);
    let refused = app
        .admin(
            Method::PATCH,
            &format!("/api/v1/webhooks/{id}"),
            Some(json!({ "url": "https://192.168.1.1/hook" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    // Rotating: a new secret, the old one signing beside it for the overlap.
    let rotated = app
        .admin(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/rotate-secret"),
            Some(json!({ "overlap_seconds": 3600 })),
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    let new_secret = rotated.json["secret"].as_str().unwrap();
    assert_ne!(new_secret, secret);
    let until = rfc3339_secs(&rotated.json["webhook"]["previous_secret_valid_until"]);
    assert!(
        (until - unix_now() as i64 - 3600).abs() < 60,
        "{}",
        rotated.text
    );
    let stored = app
        .storage
        .get_endpoint(&id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.signing_secrets(SystemTime::now()),
        vec![new_secret, secret.as_str()]
    );
    let too_long = app
        .admin(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/rotate-secret"),
            Some(json!({ "overlap_seconds": 8 * 24 * 3600 })),
        )
        .await;
    assert_eq!(
        too_long.status,
        StatusCode::BAD_REQUEST,
        "{}",
        too_long.text
    );
    let at_once = app
        .admin(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/rotate-secret"),
            Some(json!({ "overlap_seconds": 0 })),
        )
        .await;
    assert!(at_once.json["webhook"]["previous_secret_valid_until"].is_null());

    // The catalogue.
    let types = app
        .admin(Method::GET, "/api/v1/webhooks/event-types?limit=1000", None)
        .await;
    assert_eq!(types.status, StatusCode::OK, "{}", types.text);
    let names: Vec<&str> = types
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), PlatformEvent::EVENT_TYPES.len());
    assert!(names.contains(&"call.answered"));
    assert!(types
        .json
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "call.answered" && t["category"] == "call"));

    // Without `webhook_manage`: a read-only session, and a token that names
    // something else.
    app.create_user_with_role("viewer", "read_only").await;
    let viewer = session(&app, "viewer").await;
    let refused = app
        .send_as(&viewer, Method::GET, "/api/v1/webhooks", None)
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    app.create_user_with_role("boss", "tenant_admin").await;
    let boss = session(&app, "boss").await;
    let (_, narrow) = make_token(&app, &boss, json!(["user_read"])).await;
    let refused = app
        .send_as(&narrow, Method::GET, "/api/v1/webhooks", None)
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let (_, hooks) = make_token(&app, &boss, json!(["webhook_manage"])).await;
    let allowed = app
        .send_as(&hooks, Method::GET, "/api/v1/webhooks", None)
        .await;
    assert_eq!(allowed.status, StatusCode::OK, "{}", allowed.text);

    let gone = app
        .admin(Method::DELETE, &format!("/api/v1/webhooks/{id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    let gone = app
        .admin(Method::GET, &format!("/api/v1/webhooks/{id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.text);

    app.finish().await;
}

/// A delivery is signed over its time and body, verifies under the secret,
/// fails as stale later, is logged, replays signed anew, is signed twice
/// while a rotation overlaps, and a failure says what the receiver said.
#[tokio::test]
async fn a_webhook_delivery_is_signed_logged_and_replayed() {
    let Some(app) = App::with_private_webhooks().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping PostgreSQL API handler test");
        return;
    };
    let (receiver, url) = Receiver::start().await;
    let made = app
        .admin(
            Method::POST,
            "/api/v1/webhooks",
            Some(json!({ "url": url })),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let secret = made.json["secret"].as_str().unwrap().to_string();
    let id = made.json["webhook"]["id"].as_str().unwrap().to_string();

    let tested = app
        .admin(Method::POST, &format!("/api/v1/webhooks/{id}/test"), None)
        .await;
    assert_eq!(tested.status, StatusCode::OK, "{}", tested.text);
    assert_eq!(tested.json["delivered"], true, "{}", tested.text);
    assert_eq!(tested.json["status_code"], 200);
    let (signature, event, body) = receiver.last();
    assert_eq!(event, "webhook.test");
    assert!(
        verifies(&secret, &signature, &body, unix_now()),
        "{signature}"
    );
    assert!(!verifies("whsec_wrong", &signature, &body, unix_now()));
    assert!(
        !verifies(&secret, &signature, &body, unix_now() + 600),
        "a delivery replayed ten minutes later is refused as stale"
    );
    assert!(!verifies(
        &secret,
        &signature,
        &body.replace("test", "tset"),
        unix_now()
    ));
    let payload: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(payload["event_type"], "webhook.test");
    assert_eq!(payload["tenant_id"], app.tenant_a.to_string());
    assert_eq!(payload["data"]["webhook_id"], id);

    // Logged, and replayed signed now.
    let deliveries = app
        .admin(
            Method::GET,
            &format!("/api/v1/webhooks/{id}/deliveries"),
            None,
        )
        .await;
    assert_eq!(deliveries.status, StatusCode::OK, "{}", deliveries.text);
    let first = &deliveries.json[0];
    assert_eq!(first["event_type"], "webhook.test");
    assert_eq!(first["delivered"], true);
    assert_eq!(first["payload"]["event_id"], payload["event_id"]);
    let delivery = first["id"].as_str().unwrap().to_string();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let replayed = app
        .admin(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/deliveries/{delivery}/replay"),
            None,
        )
        .await;
    assert_eq!(replayed.status, StatusCode::OK, "{}", replayed.text);
    assert_eq!(replayed.json["delivered"], true);
    assert_eq!(receiver.count(), 2);
    let (again, _, again_body) = receiver.last();
    assert_eq!(again_body, body, "the same delivery");
    assert!(
        signature_parts(&again).0 > signature_parts(&signature).0,
        "signed anew"
    );
    assert!(verifies(&secret, &again, &again_body, unix_now()));
    let deliveries = app
        .admin(
            Method::GET,
            &format!("/api/v1/webhooks/{id}/deliveries"),
            None,
        )
        .await;
    assert_eq!(deliveries.json[0]["attempts"], 2, "{}", deliveries.text);
    // Another tenant cannot replay it.
    let other = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            &format!("/api/v1/webhooks/{id}/deliveries/{delivery}/replay"),
            None,
        )
        .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "{}", other.text);

    // While a rotation overlaps, both secrets sign.
    let rotated = app
        .admin(
            Method::POST,
            &format!("/api/v1/webhooks/{id}/rotate-secret"),
            None,
        )
        .await;
    let new_secret = rotated.json["secret"].as_str().unwrap().to_string();
    app.admin(Method::POST, &format!("/api/v1/webhooks/{id}/test"), None)
        .await;
    let (signature, _, body) = receiver.last();
    assert_eq!(signature_parts(&signature).1.len(), 2, "{signature}");
    assert!(verifies(&new_secret, &signature, &body, unix_now()));
    assert!(verifies(&secret, &signature, &body, unix_now()));

    // A failure says what the receiver answered.
    receiver
        .answer
        .store(500, std::sync::atomic::Ordering::SeqCst);
    let failed = app
        .admin(Method::POST, &format!("/api/v1/webhooks/{id}/test"), None)
        .await;
    assert_eq!(failed.status, StatusCode::OK, "{}", failed.text);
    assert_eq!(failed.json["delivered"], false);
    assert_eq!(failed.json["status_code"], 500);

    // A disabled webhook is not tested.
    app.admin(
        Method::PATCH,
        &format!("/api/v1/webhooks/{id}"),
        Some(json!({ "enabled": false })),
    )
    .await;
    let disabled = app
        .admin(Method::POST, &format!("/api/v1/webhooks/{id}/test"), None)
        .await;
    assert_eq!(disabled.status, StatusCode::CONFLICT, "{}", disabled.text);

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Sessions as rows (docs/IDENTITY_AND_DEVICES.md phase 1)
// ---------------------------------------------------------------------------

/// The refresh cookie a response sets, as `fcp_refresh=<value>`.
fn refresh_cookie(res: &Response) -> Option<String> {
    res.headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("fcp_refresh="))
        .map(|v| v.split(';').next().unwrap().to_string())
}

impl App {
    async fn with_cookie(&self, method: Method, path: &str, cookie: &str) -> Response {
        self.send(method, path, &[("cookie", cookie)], None).await
    }
}

#[tokio::test]
async fn a_session_is_a_row_refreshed_by_its_cookie_and_revoked() {
    let app = require_app!();
    app.create_user_with_role("alice", "tenant_admin").await;

    // Signing in: a short access token, and the refresh secret in a cookie
    // that script cannot read and only the sign-in routes are sent.
    let signed_in = app.login("alice", PASSWORD).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
    assert_eq!(signed_in.json["expires_in"], 900);
    let set = signed_in
        .headers
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    for part in [
        "fcp_refresh=",
        "HttpOnly",
        "SameSite=Strict",
        "Path=/api/v1/auth",
    ] {
        assert!(set.contains(part), "{part} in {set}");
    }
    let token = signed_in.json["token"].as_str().unwrap().to_string();
    let cookie = refresh_cookie(&signed_in).unwrap();
    let me = app
        .send_as(&token, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.json["username"], "alice", "{}", me.text);

    // Refresh: a new access token, the secret turned over.
    let refreshed = app
        .with_cookie(Method::POST, "/api/v1/auth/refresh", &cookie)
        .await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.text);
    let turned = refresh_cookie(&refreshed).expect("a new secret");
    assert_ne!(turned, cookie);
    // The secret before it, again at once (a second tab): taken, nothing
    // turned over.
    let again = app
        .with_cookie(Method::POST, "/api/v1/auth/refresh", &cookie)
        .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert!(refresh_cookie(&again).is_none());

    // The previous secret after the grace: two holders, so the session ends.
    let sessions = app.dyn_storage.login_session_store().unwrap().clone();
    let (sid, secret) =
        fcp_core::session::parse_refresh_cookie(cookie.trim_start_matches("fcp_refresh=")).unwrap();
    let later = std::time::SystemTime::now() + Duration::from_secs(60);
    let outcome = sessions
        .rotate_refresh(&sid, &fcp_core::session::refresh_hash(&secret), "x", later)
        .await
        .unwrap();
    assert_eq!(outcome, fcp_core::session::RefreshOutcome::Reused);
    let refused = app
        .send_as(&token, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNAUTHORIZED,
        "a revoked session's token"
    );
    let refused = app
        .with_cookie(Method::POST, "/api/v1/auth/refresh", &turned)
        .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);

    // Signing out ends the session; the cookie is cleared.
    let signed_in = app.login("alice", PASSWORD).await;
    let token = signed_in.json["token"].as_str().unwrap().to_string();
    let cookie = refresh_cookie(&signed_in).unwrap();
    let out = app
        .with_cookie(Method::POST, "/api/v1/auth/logout", &cookie)
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(refresh_cookie(&out).is_some_and(|c| c == "fcp_refresh="));
    assert_eq!(
        app.send_as(&token, Method::GET, "/api/v1/auth/me", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // A password change signs the user out everywhere else.
    let here = app.login("alice", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let elsewhere = app.login("alice", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let changed = app
        .send_as(
            &here,
            Method::PUT,
            "/api/v1/me/password",
            Some(json!({"current_password": PASSWORD, "new_password": "An0ther-Passw0rd!"})),
        )
        .await;
    assert!(changed.status.is_success(), "{}", changed.text);
    let me = app
        .send_as(&here, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.status, StatusCode::OK, "this session stays: {}", me.text);
    let gone = app
        .send_as(&elsewhere, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(
        gone.status,
        StatusCode::UNAUTHORIZED,
        "another session ends"
    );

    // Disabled: signed out; enabled again, still signed out.
    app.admin(
        Method::PUT,
        "/api/v1/users/alice",
        Some(json!({"enabled": false})),
    )
    .await;
    app.admin(
        Method::PUT,
        "/api/v1/users/alice",
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(
        app.send_as(&here, Method::GET, "/api/v1/auth/me", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // A sign-in is audited under the user and their tenant.
    let entry = wait_for_audit(
        &app,
        "/api/v1/audit-log?action=POST%20/api/v1/auth/login",
        |e| e["status"] == 200 && e["actor"] == "alice",
    )
    .await;
    assert_eq!(entry["actor_kind"], "sign_in", "{entry}");
    assert_eq!(entry["tenant_id"], app.tenant_a.to_string(), "{entry}");

    app.finish().await;
}

#[tokio::test]
async fn a_user_made_again_with_the_same_name_inherits_nothing() {
    let app = require_app!();
    let first_id = app.create_user_with_role("bob", "tenant_admin").await;

    // The first Bob's session, and a personal token he made.
    let session = app.login("bob", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let made = app
        .send_as(
            &session,
            Method::POST,
            "/api/v1/me/api-tokens",
            Some(json!({"name": "bob-phone"})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let personal = made.json["secret"].as_str().unwrap().to_string();
    assert_eq!(
        made.json["token"]["created_by"], "bob",
        "shown by name: {}",
        made.text
    );
    assert_eq!(
        app.send_as(&personal, Method::GET, "/api/v1/me/profile", None)
            .await
            .status,
        StatusCode::OK
    );

    // By id, in his tenant only.
    let by_id = app
        .admin(
            Method::GET,
            &format!("/api/v1/users/by-id/{first_id}"),
            None,
        )
        .await;
    assert_eq!(by_id.json["username"], "bob", "{}", by_id.text);
    assert_eq!(
        app.admin_in(
            app.tenant_b,
            Method::GET,
            &format!("/api/v1/users/by-id/{first_id}"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );

    // Bob is deleted and purged, and another Bob is made.
    app.admin(Method::DELETE, "/api/v1/users/bob", None).await;
    let purged = app
        .admin(Method::POST, "/api/v1/users/bob/purge", None)
        .await;
    assert_eq!(purged.status, StatusCode::OK, "{}", purged.text);
    let second_id = app.create_user_with_role("bob", "tenant_admin").await;
    assert_ne!(first_id, second_id);
    assert_eq!(
        app.send_as(&session, Method::GET, "/api/v1/auth/me", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the first Bob's session is not the second's"
    );
    assert_eq!(
        app.send_as(&personal, Method::GET, "/api/v1/me/profile", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "nor is his token"
    );

    // A username shaped like an id is refused.
    let refused = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(
                json!({"username": format!("user-{}", uuid::Uuid::new_v4()), "password": PASSWORD}),
            ),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    // A token signed before sessions were rows still works until it
    // expires: its subject is the username.
    let legacy = fcp_core::session::SessionKeys::new(
        fcp_core::session::SessionKey::derived_from_auth_token(ADMIN_TOKEN),
        vec![],
    );
    let now = fcp_core::session::epoch(std::time::SystemTime::now());
    let old = legacy.encode(&fcp_core::session::SessionClaims {
        sub: "bob".into(),
        sid: None,
        username: None,
        role: "tenant_admin".into(),
        tenant_id: app.tenant_a.to_string(),
        display_name: None,
        exp: now + 600,
        iat: now,
        purpose: None,
    });
    let me = app
        .send_as(&old, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.json["username"], "bob", "{}", me.text);

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Login domains and the password policy (docs/IDENTITY_AND_DEVICES.md phase 2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tenants_people_find_it_by_their_email_domain() {
    let app = require_app!();
    let tenant_path = format!("/api/v1/tenants/{}", app.tenant_a);

    // A platform admin claims a domain for tenant A; tenant B cannot claim
    // it too, and nothing that is not a domain is taken.
    let set = app
        .admin(
            Method::PUT,
            &tenant_path,
            Some(json!({"login_domains": ["ACME.example"]})),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(set.json["login_domains"], json!(["acme.example"]));
    let taken = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_b),
            Some(json!({"login_domains": ["acme.example"]})),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);
    let bad = app
        .admin(
            Method::PUT,
            &tenant_path,
            Some(json!({"login_domains": ["10.0.0.1"]})),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    // A tenant admin may not: a domain decides where people sign in.
    app.create_user_with_role("owner", "tenant_admin").await;
    let owner = app.login("owner", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = app
        .send_as(
            &owner,
            Method::PUT,
            &tenant_path,
            Some(json!({"login_domains": ["evil.example"]})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);

    // Discovery: an email at the domain finds the tenant; anything else
    // finds nothing, and nobody learns whether a user exists.
    let found = app
        .send(
            Method::POST,
            "/api/v1/auth/discover",
            &[],
            Some(json!({"login": "jane@Acme.Example"})),
        )
        .await;
    assert_eq!(found.status, StatusCode::OK, "{}", found.text);
    assert_eq!(found.json["tenant_id"], app.tenant_a.to_string());
    assert_eq!(found.json["methods"], json!(["password"]));
    for login in ["jane", "jane@elsewhere.example"] {
        let none = app
            .send(
                Method::POST,
                "/api/v1/auth/discover",
                &[],
                Some(json!({"login": login})),
            )
            .await;
        assert!(
            none.json.get("tenant_id").is_none(),
            "{login}: {}",
            none.text
        );
        assert_eq!(none.json["methods"], json!(["password"]));
    }

    // Jane signs in with her email: by her email field, in tenant A, though
    // a Jane in tenant B shares her username.
    let made = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "jane", "password": PASSWORD, "email": "jane@acme.example"})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let other = app
        .admin_in(
            app.tenant_b,
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "jane", "password": PASSWORD})),
        )
        .await;
    assert_eq!(other.status, StatusCode::CREATED, "{}", other.text);
    let by_email = app.login("jane@acme.example", PASSWORD).await;
    assert_eq!(by_email.status, StatusCode::OK, "{}", by_email.text);
    assert_eq!(by_email.json["user"]["tenant_id"], app.tenant_a.to_string());
    assert_eq!(by_email.json["user"]["username"], "jane");
    let wrong = app.login("jane@acme.example", "Wr0ng-Passw0rd!").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    // A user made without a password has none to sign in with, and a SIP
    // password of their own.
    let bare = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "sso-only"})),
        )
        .await;
    assert_eq!(bare.status, StatusCode::CREATED, "{}", bare.text);
    let stored = fcp_core::CredentialStore::get_user_in_tenant(
        app.storage.as_ref(),
        "sso-only",
        &app.tenant_a,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(stored.password.is_empty());
    assert!(stored.sip_ha1_hash.is_some() && stored.digest_password.is_some());
    for guess in ["", PASSWORD] {
        let res = app.login("sso-only", guess).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{}", res.text);
    }

    app.finish().await;
}

#[tokio::test]
async fn a_tenants_password_policy_is_kept_and_never_shuts_out_a_super_admin() {
    let app = require_app!();
    app.create_user_with_role("ann", "end_user").await;
    app.create_user_with_role("tom", "tenant_admin").await;
    app.create_user_with_role("root", "super_admin").await;

    // Until the tenant has an identity provider, a password is the only
    // way in: the API will not take it away.
    let refused = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({"sign_in": {"password_login": "admins_only"}})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    // As a tenant with one will have it: administrators only.
    let set = |policy: fcp_core::PasswordLogin| {
        let storage = Arc::clone(&app.storage);
        let tenant = app.tenant_a;
        async move {
            let mut t = TenantStore::get_tenant(storage.as_ref(), &tenant)
                .await
                .unwrap()
                .unwrap();
            t.sign_in.password_login = policy;
            TenantStore::put_tenant(storage.as_ref(), t).await.unwrap();
        }
    };
    set(fcp_core::PasswordLogin::AdminsOnly).await;
    let ann = app.login("ann", PASSWORD).await;
    assert_eq!(ann.status, StatusCode::FORBIDDEN, "{}", ann.text);
    assert_eq!(app.login("tom", PASSWORD).await.status, StatusCode::OK);
    // A wrong password is refused as wrong: the policy is said only to
    // someone who knows the password.
    assert_eq!(
        app.login("ann", "Wr0ng-Passw0rd!").await.status,
        StatusCode::UNAUTHORIZED
    );

    // Nobody but a super admin.
    set(fcp_core::PasswordLogin::Disabled).await;
    assert_eq!(
        app.login("tom", PASSWORD).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(app.login("root", PASSWORD).await.status, StatusCode::OK);
    app.admin(
        Method::PUT,
        &format!("/api/v1/tenants/{}", app.tenant_a),
        Some(json!({"login_domains": ["policy.example"]})),
    )
    .await;
    let found = app
        .send(
            Method::POST,
            "/api/v1/auth/discover",
            &[],
            Some(json!({"login": "x@policy.example"})),
        )
        .await;
    assert_eq!(found.json["methods"], json!([]), "{}", found.text);

    app.finish().await;
}

// ---------------------------------------------------------------------------
// A second factor (docs/IDENTITY_AND_DEVICES.md phase 2b)
// ---------------------------------------------------------------------------

/// The code an app shows for `secret`, `ahead` steps from now.
fn totp_code(secret: &str, ahead: u64) -> String {
    let step = fcp_core::totp::step_at(std::time::SystemTime::now()) + ahead;
    fcp_core::totp::code_at(secret, step).unwrap()
}

impl App {
    async fn login_totp(&self, ticket: &str, body: Value) -> Response {
        let mut body = body;
        body["mfa_token"] = json!(ticket);
        self.send(Method::POST, "/api/v1/auth/login/totp", &[], Some(body))
            .await
    }
}

#[tokio::test]
async fn a_second_factor_is_set_up_asked_for_and_each_code_good_once() {
    let app = require_app!();
    app.create_user_with_role("tia", "end_user").await;
    let session = app.login("tia", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let status = app
        .send_as(&session, Method::GET, "/api/v1/me/totp", None)
        .await;
    assert_eq!(status.json["enabled"], false, "{}", status.text);

    // Set up: a secret for the app, confirmed by a code from it.
    let enrolled = app
        .send_as(&session, Method::POST, "/api/v1/me/totp/enroll", None)
        .await;
    assert_eq!(enrolled.status, StatusCode::OK, "{}", enrolled.text);
    let secret = enrolled.json["secret"].as_str().unwrap().to_string();
    assert!(enrolled.json["otpauth_uri"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/"));
    let wrong = app
        .send_as(
            &session,
            Method::POST,
            "/api/v1/me/totp/confirm",
            Some(json!({"code": "000000"})),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let confirmed = app
        .send_as(
            &session,
            Method::POST,
            "/api/v1/me/totp/confirm",
            Some(json!({"code": totp_code(&secret, 0)})),
        )
        .await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text);
    let recovery: Vec<String> = confirmed.json["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    assert_eq!(recovery.len(), 10);

    // Signing in now wants a code: a ticket, not a session.
    let first = app.login("tia", PASSWORD).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(first.json["mfa_required"], "totp");
    assert!(first.json.get("token").is_none(), "{}", first.text);
    assert!(refresh_cookie(&first).is_none());
    let ticket = first.json["mfa_token"].as_str().unwrap().to_string();
    assert_eq!(
        app.send_as(&ticket, Method::GET, "/api/v1/auth/me", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "a ticket is not a session"
    );
    let bad = app.login_totp(&ticket, json!({"code": "123456"})).await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    // The code the set-up used is spent; the next step's is good, once.
    let next = totp_code(&secret, 1);
    let ok = app.login_totp(&ticket, json!({"code": next})).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert!(ok.json["token"].is_string());
    let replay = app.login_totp(&ticket, json!({"code": next})).await;
    assert_eq!(
        replay.status,
        StatusCode::UNAUTHORIZED,
        "a code is good once"
    );

    // A recovery code stands in for the phone, once.
    let rescued = app
        .login_totp(
            &ticket,
            json!({"recovery_code": recovery[0].to_uppercase()}),
        )
        .await;
    assert_eq!(rescued.status, StatusCode::OK, "{}", rescued.text);
    let again = app
        .login_totp(&ticket, json!({"recovery_code": recovery[0]}))
        .await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    // An administrator resets it (a lost phone): a password signs in.
    let reset = app
        .admin(Method::DELETE, "/api/v1/users/tia/totp", None)
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT, "{}", reset.text);
    let plain = app.login("tia", PASSWORD).await;
    assert!(plain.json["token"].is_string(), "{}", plain.text);

    app.finish().await;
}

#[tokio::test]
async fn wrong_codes_lock_the_account_as_wrong_passwords_do() {
    let app = require_app!();
    app.create_user_with_role("lou", "end_user").await;
    let session = app.login("lou", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let secret = app
        .send_as(&session, Method::POST, "/api/v1/me/totp/enroll", None)
        .await
        .json["secret"]
        .as_str()
        .unwrap()
        .to_string();
    app.send_as(
        &session,
        Method::POST,
        "/api/v1/me/totp/confirm",
        Some(json!({"code": totp_code(&secret, 0)})),
    )
    .await;

    // A right password does not reset the count the wrong codes add to.
    for _ in 0..5 {
        let ticket = app.login("lou", PASSWORD).await.json["mfa_token"]
            .as_str()
            .unwrap()
            .to_string();
        let bad = app.login_totp(&ticket, json!({"code": "000000"})).await;
        assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    }
    let locked = app.login("lou", PASSWORD).await;
    assert_eq!(locked.status, StatusCode::FORBIDDEN, "{}", locked.text);

    app.finish().await;
}

#[tokio::test]
async fn a_tenant_requiring_a_second_factor_has_its_admins_set_one_up_signing_in() {
    let app = require_app!();
    let set = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({"sign_in": {"password_login": "allowed", "require_totp": "admins"}})),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(set.json["sign_in"]["require_totp"], "admins");

    // An end user is not asked.
    app.create_user_with_role("eve", "end_user").await;
    assert!(app.login("eve", PASSWORD).await.json["token"].is_string());

    // An admin without one sets it up before they are in.
    app.create_user_with_role("ada", "tenant_admin").await;
    let first = app.login("ada", PASSWORD).await;
    assert_eq!(
        first.json["mfa_required"], "totp_enrollment",
        "{}",
        first.text
    );
    let ticket = first.json["mfa_token"].as_str().unwrap().to_string();
    let enrolled = app
        .send(
            Method::POST,
            "/api/v1/auth/totp/enroll",
            &[],
            Some(json!({"mfa_token": ticket})),
        )
        .await;
    assert_eq!(enrolled.status, StatusCode::OK, "{}", enrolled.text);
    let secret = enrolled.json["secret"].as_str().unwrap().to_string();
    let done = app
        .send(
            Method::POST,
            "/api/v1/auth/totp/confirm",
            &[],
            Some(json!({"mfa_token": ticket, "code": totp_code(&secret, 0)})),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text);
    assert!(done.json["token"].is_string());
    assert_eq!(done.json["recovery_codes"].as_array().unwrap().len(), 10);
    let session = done.json["token"].as_str().unwrap().to_string();

    // And cannot turn it off.
    let off = app
        .send_as(
            &session,
            Method::POST,
            "/api/v1/me/totp/disable",
            Some(json!({"code": totp_code(&secret, 1)})),
        )
        .await;
    assert_eq!(off.status, StatusCode::FORBIDDEN, "{}", off.text);
    let status = app
        .send_as(&session, Method::GET, "/api/v1/me/totp", None)
        .await;
    assert_eq!(status.json["required"], true, "{}", status.text);
    assert_eq!(status.json["enabled"], true);

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Single sign-on (docs/IDENTITY_AND_DEVICES.md phase 3)
// ---------------------------------------------------------------------------

use fcp_testkit::oidc::StandInIdp;

const IDP_CLIENT: &str = "fcp-client";
const IDP_SECRET: &str = "idp-s3cret";

fn location(res: &Response) -> String {
    res.headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

fn query_param(url: &str, name: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

impl App {
    /// A provider for tenant A at the stand-in, with `settings` over the
    /// usual ones.
    async fn make_provider(&self, idp: &StandInIdp, settings: Value) -> String {
        let mut base = json!({"issuer": idp.issuer, "client_id": IDP_CLIENT});
        base.as_object_mut()
            .unwrap()
            .extend(settings.as_object().unwrap().clone());
        let res = self
            .admin(
                Method::POST,
                "/api/v1/identity-providers",
                Some(json!({"name": "Acme SSO", "settings": base, "client_secret": IDP_SECRET})),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
        res.json["id"].as_str().unwrap().to_string()
    }

    /// Begin a sign-in through `provider`: the provider's authorize URL and
    /// the state cookie.
    async fn sso_start(&self, provider: &str) -> (String, String) {
        let res = self
            .send(
                Method::GET,
                &format!("/api/v1/auth/sso/{provider}/start"),
                &[],
                None,
            )
            .await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.text);
        let cookie = res
            .headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("fcp_sso="))
            .map(|v| v.split(';').next().unwrap().to_string())
            .expect("the state cookie");
        (location(&res), cookie)
    }

    /// The provider's answer to `authorize`: the callback path and query.
    async fn at_provider(&self, authorize: &str) -> String {
        let back = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(authorize)
            .send()
            .await
            .unwrap();
        let to = back.headers()[header::LOCATION.as_str()]
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            to.starts_with(&format!("{SSO_PUBLIC_URL}/api/v1/auth/sso/callback?")),
            "{to}"
        );
        to.trim_start_matches(SSO_PUBLIC_URL).to_string()
    }

    /// Sign in through `provider` as `claims`: where the login page is
    /// sent (`sso_code` or `sso_error`).
    async fn sso_landing(&self, idp: &StandInIdp, provider: &str, claims: Value) -> String {
        idp.sign_in_as(claims);
        let (authorize, cookie) = self.sso_start(provider).await;
        let callback = self.at_provider(&authorize).await;
        let res = self.with_cookie(Method::GET, &callback, &cookie).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.text);
        let landing = location(&res);
        assert!(
            landing.starts_with(&format!("{SSO_PUBLIC_URL}/login?")),
            "{landing}"
        );
        landing
    }

    /// Exchange a landing's one-time code.
    async fn sso_exchange(&self, landing: &str) -> Response {
        let code = query_param(landing, "sso_code").unwrap_or_default();
        self.send(
            Method::POST,
            "/api/v1/auth/sso/exchange",
            &[],
            Some(json!({"code": code})),
        )
        .await
    }
}

#[tokio::test]
async fn a_tenants_people_sign_in_through_its_identity_provider() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    let set = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({"login_domains": ["acme.test"]})),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let provider = app
        .make_provider(
            &idp,
            json!({"jit": true, "group_roles": [{"group": "fcp-admins", "role": "tenant_admin"}]}),
        )
        .await;

    // The secret is kept, never shown; the redirect URI to register is.
    let read = app
        .admin(
            Method::GET,
            &format!("/api/v1/identity-providers/{provider}"),
            None,
        )
        .await;
    assert_eq!(read.json["has_client_secret"], true, "{}", read.text);
    assert!(!read.text.contains(IDP_SECRET));
    assert_eq!(
        read.json["redirect_uri"],
        format!("{SSO_PUBLIC_URL}/api/v1/auth/sso/callback")
    );
    let checked = app
        .admin(
            Method::POST,
            &format!("/api/v1/identity-providers/{provider}/check"),
            None,
        )
        .await;
    assert_eq!(checked.json["ok"], true, "{}", checked.text);

    // The login page offers it to the tenant's people.
    let found = app
        .send(
            Method::POST,
            "/api/v1/auth/discover",
            &[],
            Some(json!({"login": "jane@acme.test"})),
        )
        .await;
    assert_eq!(
        found.json["methods"],
        json!(["password", "sso"]),
        "{}",
        found.text
    );
    assert_eq!(found.json["providers"][0]["id"], provider.as_str());

    // First sign-in: Jane is made, an end user, named from her address.
    let jane = json!({
        "sub": "jane-1", "email": "jane@acme.test", "email_verified": true,
        "preferred_username": "jane@acme.test", "name": "Jane Doe", "groups": ["staff"],
    });
    let landing = app.sso_landing(&idp, &provider, jane.clone()).await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
    assert_eq!(signed_in.json["user"]["username"], "jane");
    assert_eq!(signed_in.json["user"]["role"], "end_user");
    assert!(refresh_cookie(&signed_in).is_some());
    let token = signed_in.json["token"].as_str().unwrap().to_string();
    let me = app
        .send_as(&token, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.json["username"], "jane", "{}", me.text);

    // The one-time code is good once.
    let again = app.sso_exchange(&landing).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    // Her groups decide her role, at every sign-in; she is the same user
    // however the provider names her now.
    let mut promoted = jane.clone();
    promoted["groups"] = json!(["staff", "fcp-admins"]);
    promoted["preferred_username"] = json!("jane.doe@acme.test");
    let landing = app.sso_landing(&idp, &provider, promoted).await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(
        signed_in.json["user"]["username"], "jane",
        "{}",
        signed_in.text
    );
    assert_eq!(signed_in.json["user"]["role"], "tenant_admin");

    let identities = app
        .admin(Method::GET, "/api/v1/users/jane/identities", None)
        .await;
    assert_eq!(
        identities.json[0]["subject"], "jane-1",
        "{}",
        identities.text
    );
    assert_eq!(identities.json[0]["provider_id"], provider.as_str());

    app.finish().await;
}

#[tokio::test]
async fn a_sign_in_through_a_provider_is_refused_when_it_should_be() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    let provider = app.make_provider(&idp, json!({"jit": false})).await;
    let error = |landing: &str| query_param(landing, "sso_error").unwrap_or_default();

    // An existing user is linked by the email the provider has verified.
    app.admin(
        Method::POST,
        "/api/v1/users",
        Some(json!({"username": "bob", "email": "bob@acme.test"})),
    )
    .await;
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "b", "email": "bob@acme.test", "email_verified": true}),
        )
        .await;
    assert_eq!(
        app.sso_exchange(&landing).await.json["user"]["username"],
        "bob",
        "{landing}"
    );

    // An unverified email links nobody; with no creation, no way in.
    app.admin(
        Method::POST,
        "/api/v1/users",
        Some(json!({"username": "carol", "email": "carol@acme.test"})),
    )
    .await;
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "c", "email": "carol@acme.test"}),
        )
        .await;
    assert_eq!(error(&landing), "not_provisioned");

    // A callback this browser did not begin.
    idp.sign_in_as(json!({"sub": "b"}));
    let (authorize, _cookie) = app.sso_start(&provider).await;
    let callback = app.at_provider(&authorize).await;
    let res = app.send(Method::GET, &callback, &[], None).await;
    assert_eq!(error(&location(&res)), "bad_state");
    let (_, other_cookie) = app.sso_start(&provider).await;
    let res = app.with_cookie(Method::GET, &callback, &other_cookie).await;
    assert_eq!(error(&location(&res)), "bad_state");

    // A token for another client, and a provider that turns us away.
    idp.override_claims(json!({"aud": "someone-else"}));
    let landing = app.sso_landing(&idp, &provider, json!({"sub": "b"})).await;
    assert_eq!(error(&landing), "bad_id_token");
    idp.override_claims(json!({}));
    let wrong_secret = app
        .admin(
            Method::PUT,
            &format!("/api/v1/identity-providers/{provider}"),
            Some(json!({
                "name": "Acme SSO",
                "settings": {"issuer": idp.issuer, "client_id": IDP_CLIENT},
                "client_secret": "wrong",
            })),
        )
        .await;
    assert_eq!(wrong_secret.status, StatusCode::OK, "{}", wrong_secret.text);
    let landing = app.sso_landing(&idp, &provider, json!({"sub": "b"})).await;
    assert_eq!(error(&landing), "provider_refused");

    // A disabled user does not get in by their provider either.
    app.admin(
        Method::PUT,
        &format!("/api/v1/identity-providers/{provider}"),
        Some(json!({
            "name": "Acme SSO",
            "settings": {"issuer": idp.issuer, "client_id": IDP_CLIENT},
            "client_secret": IDP_SECRET,
        })),
    )
    .await;
    app.admin(
        Method::PUT,
        "/api/v1/users/bob",
        Some(json!({"enabled": false})),
    )
    .await;
    let landing = app.sso_landing(&idp, &provider, json!({"sub": "b"})).await;
    assert_eq!(error(&landing), "disabled");

    // A platform administrator is never signed in by a tenant's provider,
    // even one asserting their verified email.
    app.admin(
        Method::POST,
        "/api/v1/users",
        Some(json!({"username": "root2", "password": PASSWORD, "role": "super_admin", "email": "root@acme.test"})),
    )
    .await;
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "r", "email": "root@acme.test", "email_verified": true}),
        )
        .await;
    assert_eq!(error(&landing), "super_admin");

    // A tenant's provider grants no platform role.
    let root = app
        .admin(
            Method::POST,
            "/api/v1/identity-providers",
            Some(
                json!({"name": "x", "settings": {"issuer": idp.issuer, "client_id": "c",
                "group_roles": [{"group": "g", "role": "super_admin"}]}}),
            ),
        )
        .await;
    assert_eq!(root.status, StatusCode::BAD_REQUEST, "{}", root.text);

    app.finish().await;
}

#[tokio::test]
async fn passwords_can_be_kept_from_a_tenant_once_it_has_a_provider() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    let tenant_path = format!("/api/v1/tenants/{}", app.tenant_a);
    let policy = |p: &str| json!({"sign_in": {"password_login": p}});

    let early = app
        .admin(Method::PUT, &tenant_path, Some(policy("disabled")))
        .await;
    assert_eq!(early.status, StatusCode::BAD_REQUEST, "{}", early.text);

    let provider = app.make_provider(&idp, json!({"jit": true})).await;
    let set = app
        .admin(Method::PUT, &tenant_path, Some(policy("disabled")))
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);

    app.create_user_with_role("pat", "end_user").await;
    let refused = app.login("pat", PASSWORD).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "p", "preferred_username": "pat2"}),
        )
        .await;
    assert_eq!(
        app.sso_exchange(&landing).await.status,
        StatusCode::OK,
        "{landing}"
    );

    // The last way in cannot be taken away while passwords are kept out.
    let path = format!("/api/v1/identity-providers/{provider}");
    let gone = app.admin(Method::DELETE, &path, None).await;
    assert_eq!(gone.status, StatusCode::CONFLICT, "{}", gone.text);
    let off = app
        .admin(
            Method::PUT,
            &path,
            Some(json!({"name": "Acme SSO", "enabled": false,
                "settings": {"issuer": idp.issuer, "client_id": IDP_CLIENT}})),
        )
        .await;
    assert_eq!(off.status, StatusCode::CONFLICT, "{}", off.text);
    app.admin(Method::PUT, &tenant_path, Some(policy("allowed")))
        .await;
    assert_eq!(
        app.admin(Method::DELETE, &path, None).await.status,
        StatusCode::NO_CONTENT
    );

    app.finish().await;
}

#[tokio::test]
async fn a_providers_back_channel_logout_ends_the_sessions_it_names() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    let provider = app.make_provider(&idp, json!({"jit": true})).await;
    let shown = app
        .admin(
            Method::GET,
            &format!("/api/v1/identity-providers/{provider}"),
            None,
        )
        .await;
    let path = format!("/api/v1/auth/sso/{provider}/backchannel-logout");
    assert_eq!(
        shown.json["backchannel_logout_uri"],
        format!("{SSO_PUBLIC_URL}{path}"),
        "{}",
        shown.text
    );

    // Two sign-ins, two of the provider's sessions.
    let mut sessions = Vec::new();
    for sid in ["s-1", "s-2"] {
        let landing = app
            .sso_landing(
                &idp,
                &provider,
                json!({"sub": "kim-1", "preferred_username": "kim", "sid": sid}),
            )
            .await;
        let signed_in = app.sso_exchange(&landing).await;
        assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
        sessions.push(refresh_cookie(&signed_in).unwrap());
    }
    let refreshes = |cookie: String| {
        let app = &app;
        async move {
            app.with_cookie(Method::POST, "/api/v1/auth/refresh", &cookie)
                .await
                .status
        }
    };
    let logout = |token: String| {
        let app = &app;
        let path = path.clone();
        async move {
            app.send_raw(
                "unused",
                Method::POST,
                &path,
                "application/x-www-form-urlencoded",
                format!("logout_token={token}").into_bytes(),
            )
            .await
        }
    };

    // A forged token ends nothing.
    let forged =
        logout(idp.sign(&json!({"iss": idp.issuer, "aud": IDP_CLIENT, "sid": "s-1"}))).await;
    assert_eq!(forged.status, StatusCode::BAD_REQUEST, "{}", forged.text);
    assert_eq!(refreshes(sessions[0].clone()).await, StatusCode::OK);

    // The provider's session s-1 ends; s-2 goes on.
    let one = logout(idp.logout_token(None, Some("s-1"))).await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    assert_eq!(one.headers["cache-control"], "no-store");
    assert_eq!(
        refreshes(sessions[0].clone()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(refreshes(sessions[1].clone()).await, StatusCode::OK);

    // The person signs out at the provider: every session of theirs ends.
    let all = logout(idp.logout_token(Some("kim-1"), None)).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    assert_eq!(
        refreshes(sessions[1].clone()).await,
        StatusCode::UNAUTHORIZED
    );

    app.finish().await;
}

#[tokio::test]
async fn a_provider_signs_in_only_the_people_its_tenant_meant() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    // A Google Workspace: only the domain's accounts; Entra's verified
    // email claim.
    let provider = app
        .make_provider(
            &idp,
            json!({
                "jit": false,
                "required_claims": [{"claim": "hd", "value": "acme.test"}],
                "email_verified_claim": "xms_edov",
            }),
        )
        .await;
    app.admin(
        Method::POST,
        "/api/v1/users",
        Some(json!({"username": "lee", "email": "lee@acme.test"})),
    )
    .await;
    let error = |landing: &str| query_param(landing, "sso_error").unwrap_or_default();

    let personal = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "l", "email": "lee@acme.test", "xms_edov": true, "hd": "gmail.com"}),
        )
        .await;
    assert_eq!(error(&personal), "claim_mismatch");
    // `email_verified` is not the claim this provider verifies with.
    let unverified = app
        .sso_landing(&idp, &provider, json!({"sub": "l", "email": "lee@acme.test", "email_verified": true, "hd": "acme.test"}))
        .await;
    assert_eq!(error(&unverified), "not_provisioned");
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "l", "email": "lee@acme.test", "xms_edov": true, "hd": "acme.test"}),
        )
        .await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(
        signed_in.json["user"]["username"], "lee",
        "{landing} {}",
        signed_in.text
    );

    app.finish().await;
}

#[tokio::test]
async fn a_site_signs_its_people_in_but_neither_makes_them_nor_sets_their_roles() {
    let Some(app) = App::as_site().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    // At a site the provider arrives by the link; the site's admin may not
    // make one.
    let refused = app
        .admin(
            Method::POST,
            "/api/v1/identity-providers",
            Some(json!({"name": "x", "settings": {"issuer": idp.issuer, "client_id": IDP_CLIENT}})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let now = std::time::SystemTime::now();
    let provider = fcp_core::identity::IdentityProvider {
        id: fcp_core::identity::IdentityProviderId::new(),
        tenant_id: app.tenant_a,
        name: "Acme SSO".to_string(),
        kind: fcp_core::identity::ProviderKind::Oidc,
        enabled: true,
        settings: serde_json::from_value(json!({
            "issuer": idp.issuer, "client_id": IDP_CLIENT, "jit": true,
            "group_roles": [{"group": "it", "role": "tenant_admin"}],
        }))
        .unwrap(),
        client_secret: Some(IDP_SECRET.to_string()),
        created_at: now,
        updated_at: now,
    };
    fcp_core::identity::IdentityStore::put_identity_provider(app.storage.as_ref(), &provider)
        .await
        .unwrap();
    let id = provider.id.to_string();

    // A stranger is not made here, whatever the provider says.
    let landing = app
        .sso_landing(
            &idp,
            &id,
            json!({"sub": "n", "preferred_username": "newcomer"}),
        )
        .await;
    assert_eq!(
        query_param(&landing, "sso_error").unwrap_or_default(),
        "not_provisioned"
    );

    // A user the cloud made (written by the store, as the link writes)
    // signs in, and keeps the role the cloud gave.
    let ana: fcp_core::User = serde_json::from_value(json!({
        "username": "ana", "password": "", "enabled": true,
        "created_at": 1_700_000_000, "updated_at": 1_700_000_000,
        "email": "ana@acme.test", "tenant_id": app.tenant_a, "role": "end_user",
    }))
    .unwrap();
    fcp_core::CredentialStore::put_user(app.storage.as_ref(), ana)
        .await
        .unwrap();
    let landing = app
        .sso_landing(
            &idp,
            &id,
            json!({"sub": "a", "email": "ana@acme.test", "email_verified": true, "groups": ["it"]}),
        )
        .await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(
        signed_in.json["user"]["username"], "ana",
        "{landing} {}",
        signed_in.text
    );
    assert_eq!(signed_in.json["user"]["role"], "end_user");

    app.finish().await;
}

// ---------------------------------------------------------------------------
// SAML sign-in (docs/IDENTITY_AND_DEVICES.md phase 4), with the `saml`
// feature: the test is the identity provider — a key, its certificate in
// metadata, responses written from a template and signed by xmlsec.
// ---------------------------------------------------------------------------

#[cfg(feature = "saml")]
mod saml {
    use super::*;
    use base64::Engine;
    use samael::crypto::{Crypto, CryptoProvider};
    use samael::idp::{CertificateParams, IdentityProvider, KeyType, Rsa};

    const IDP_ENTITY: &str = "https://idp.acme.test/saml";
    const IDP_SSO: &str = "https://idp.acme.test/sso";
    pub(super) const IDP_SLO: &str = "https://idp.acme.test/slo";
    const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

    /// A SAML identity provider: its key, and the metadata naming its
    /// certificate.
    pub(super) struct SamlIdp {
        key_der: Vec<u8>,
        pub metadata: String,
    }

    impl SamlIdp {
        pub fn new() -> Self {
            let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).unwrap();
            let cert = idp
                .create_certificate(&CertificateParams {
                    common_name: "idp.acme.test",
                    issuer_name: "idp.acme.test",
                    days_until_expiration: 30,
                })
                .unwrap();
            let cert = base64::engine::general_purpose::STANDARD.encode(cert.der_data());
            Self {
                key_der: idp.export_private_key_der().unwrap(),
                metadata: metadata(Some(&cert)),
            }
        }
    }

    /// Metadata with the signing certificate, or none.
    pub(super) fn metadata(cert: Option<&str>) -> String {
        let key = cert.map(|c| format!(
            r#"<md:KeyDescriptor use="signing"><ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:X509Data><ds:X509Certificate>{c}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>"#
        )).unwrap_or_default();
        format!(
            r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY}"><md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">{key}<md:SingleLogoutService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="{IDP_SLO}"/><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="{IDP_SSO}"/></md:IDPSSODescriptor></md:EntityDescriptor>"#
        )
    }

    /// An enveloped signature for xmlsec to fill, over the element `id`.
    fn signature_template(id: &str) -> String {
        format!(
            r##"<ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:SignedInfo><ds:CanonicalizationMethod Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/><ds:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/><ds:Reference URI="#{id}"><ds:Transforms><ds:Transform Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/><ds:Transform Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/></ds:Transforms><ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><ds:DigestValue></ds:DigestValue></ds:Reference></ds:SignedInfo><ds:SignatureValue></ds:SignatureValue></ds:Signature>"##
        )
    }

    /// `assertion` encrypted to the certificate (XML Encryption: AES-128-CBC,
    /// its key wrapped with RSA-OAEP), as a provider encrypts one.
    fn encrypted(assertion: &str, cert_b64: &str) -> String {
        use openssl::rand::rand_bytes;
        let cert = openssl::x509::X509::from_der(&B64.decode(cert_b64).unwrap()).unwrap();
        let rsa = cert.public_key().unwrap().rsa().unwrap();
        let (mut key, mut iv) = ([0u8; 16], [0u8; 16]);
        rand_bytes(&mut key).unwrap();
        rand_bytes(&mut iv).unwrap();
        let body = openssl::symm::encrypt(
            openssl::symm::Cipher::aes_128_cbc(),
            &key,
            Some(&iv),
            assertion.as_bytes(),
        )
        .unwrap();
        let mut wrapped = vec![0u8; rsa.size() as usize];
        let n = rsa
            .public_encrypt(&key, &mut wrapped, openssl::rsa::Padding::PKCS1_OAEP)
            .unwrap();
        wrapped.truncate(n);
        let cipher = B64.encode([iv.to_vec(), body].concat());
        let wrapped = B64.encode(wrapped);
        format!(
            r##"<saml:EncryptedAssertion><xenc:EncryptedData xmlns:xenc="http://www.w3.org/2001/04/xmlenc#" Type="http://www.w3.org/2001/04/xmlenc#Element"><xenc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/><ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><xenc:EncryptedKey><xenc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#rsa-oaep-mgf1p"/><xenc:CipherData><xenc:CipherValue>{wrapped}</xenc:CipherValue></xenc:CipherData></xenc:EncryptedKey></ds:KeyInfo><xenc:CipherData><xenc:CipherValue>{cipher}</xenc:CipherValue></xenc:CipherData></xenc:EncryptedData></saml:EncryptedAssertion>"##
        )
    }

    impl SamlIdp {
        /// A LogoutRequest from this provider, as XML; signed for POST.
        fn logout_xml(
            &self,
            slo: &str,
            name_id: Option<&str>,
            session: Option<&str>,
            signed: bool,
        ) -> String {
            let id = format!("_l{}", Uuid::new_v4().simple());
            let at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
            let name_id = name_id
                .map(|n| format!(r#"<saml:NameID Format="urn:oasis:names:tc:SAML:2.0:nameid-format:persistent">{n}</saml:NameID>"#))
                .unwrap_or_default();
            let session = session
                .map(|s| format!("<samlp:SessionIndex>{s}</samlp:SessionIndex>"))
                .unwrap_or_default();
            let signature = if signed {
                signature_template(&id)
            } else {
                String::new()
            };
            let xml = format!(
                r##"<samlp:LogoutRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{id}" Version="2.0" IssueInstant="{at}" Destination="{slo}"><saml:Issuer>{IDP_ENTITY}</saml:Issuer>{signature}{name_id}{session}</samlp:LogoutRequest>"##
            );
            if signed {
                Crypto::sign_xml(xml.as_bytes(), &self.key_der).unwrap()
            } else {
                xml
            }
        }

        /// A logout request's query string, signed as the HTTP-Redirect
        /// binding signs it.
        pub fn logout_query(
            &self,
            slo: &str,
            name_id: Option<&str>,
            session: Option<&str>,
        ) -> String {
            use std::io::Write;
            let xml = self.logout_xml(slo, name_id, session, false);
            let mut deflated = Vec::new();
            {
                let mut encoder = flate2::write::DeflateEncoder::new(
                    &mut deflated,
                    flate2::Compression::default(),
                );
                encoder.write_all(xml.as_bytes()).unwrap();
            }
            let request = urlencoding::encode(&B64.encode(deflated)).into_owned();
            let sig_alg = urlencoding::encode("http://www.w3.org/2001/04/xmldsig-more#rsa-sha256")
                .into_owned();
            let signed = format!("SAMLRequest={request}&RelayState=rs-1&SigAlg={sig_alg}");
            let key = openssl::pkey::PKey::private_key_from_der(&self.key_der).unwrap();
            let mut signer =
                openssl::sign::Signer::new(openssl::hash::MessageDigest::sha256(), &key).unwrap();
            signer.update(signed.as_bytes()).unwrap();
            let signature =
                urlencoding::encode(&B64.encode(signer.sign_to_vec().unwrap())).into_owned();
            format!("{signed}&Signature={signature}")
        }

        /// A signed logout request for the HTTP-POST binding (base64).
        pub fn logout_post(
            &self,
            slo: &str,
            name_id: Option<&str>,
            session: Option<&str>,
        ) -> String {
            B64.encode(self.logout_xml(slo, name_id, session, true))
        }
    }

    /// What a response says; each test changes what it is about.
    pub(super) struct Assertion {
        pub request_id: String,
        pub sp_entity: String,
        pub acs: String,
        pub name_id: String,
        pub name_id_format: String,
        pub assertion_id: String,
        pub attributes: Vec<(&'static str, Vec<String>)>,
        pub session_index: String,
        /// FCP's certificate (base64 DER) to encrypt the assertion to.
        pub encrypt_to: Option<String>,
    }

    impl Assertion {
        pub fn new(request_id: &str, sp_entity: &str, name_id: &str) -> Self {
            Self {
                request_id: request_id.to_string(),
                sp_entity: sp_entity.to_string(),
                acs: format!("{sp_entity}/acs"),
                name_id: name_id.to_string(),
                name_id_format: "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent".to_string(),
                assertion_id: format!("_a{}", Uuid::new_v4().simple()),
                attributes: Vec::new(),
                session_index: "idp-session-1".to_string(),
                encrypt_to: None,
            }
        }

        fn xml(&self) -> String {
            let now = chrono::Utc::now();
            let at = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            let (issued, until, before) = (
                at(now),
                at(now + chrono::Duration::minutes(5)),
                at(now - chrono::Duration::minutes(1)),
            );
            let response_id = format!("_r{}", Uuid::new_v4().simple());
            let attributes: String = self
                .attributes
                .iter()
                .map(|(name, values)| {
                    let values: String = values
                        .iter()
                        .map(|v| format!("<saml:AttributeValue>{v}</saml:AttributeValue>"))
                        .collect();
                    format!(r#"<saml:Attribute Name="{name}">{values}</saml:Attribute>"#)
                })
                .collect();
            let assertion = format!(
                r##"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{aid}" Version="2.0" IssueInstant="{issued}"><saml:Issuer>{IDP_ENTITY}</saml:Issuer><saml:Subject><saml:NameID Format="{format}">{name_id}</saml:NameID><saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer"><saml:SubjectConfirmationData NotOnOrAfter="{until}" Recipient="{acs}" InResponseTo="{req}"/></saml:SubjectConfirmation></saml:Subject><saml:Conditions NotBefore="{before}" NotOnOrAfter="{until}"><saml:AudienceRestriction><saml:Audience>{audience}</saml:Audience></saml:AudienceRestriction></saml:Conditions><saml:AuthnStatement AuthnInstant="{issued}" SessionIndex="{session}"><saml:AuthnContext><saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:Password</saml:AuthnContextClassRef></saml:AuthnContext></saml:AuthnStatement><saml:AttributeStatement>{attributes}</saml:AttributeStatement></saml:Assertion>"##,
                acs = self.acs,
                req = self.request_id,
                aid = self.assertion_id,
                format = self.name_id_format,
                name_id = self.name_id,
                audience = self.sp_entity,
                session = self.session_index,
            );
            let assertion = match &self.encrypt_to {
                Some(cert) => encrypted(&assertion, cert),
                None => assertion,
            };
            format!(
                r##"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{response_id}" Version="2.0" IssueInstant="{issued}" Destination="{acs}" InResponseTo="{req}"><saml:Issuer>{IDP_ENTITY}</saml:Issuer>{signature}<samlp:Status><samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/></samlp:Status>{assertion}</samlp:Response>"##,
                acs = self.acs,
                req = self.request_id,
                signature = signature_template(&response_id),
            )
        }

        /// Signed with `idp`'s key, base64 as the HTTP-POST binding sends it.
        pub fn signed_by(&self, idp: &SamlIdp) -> String {
            let signed = Crypto::sign_xml(self.xml().as_bytes(), &idp.key_der).unwrap();
            base64::engine::general_purpose::STANDARD.encode(signed)
        }

        /// The same, with its signature taken out.
        pub fn unsigned(&self) -> String {
            let xml = self.xml();
            let (head, rest) = xml.split_once("<ds:Signature").unwrap();
            let tail = &rest[rest.find("</ds:Signature>").unwrap() + "</ds:Signature>".len()..];
            base64::engine::general_purpose::STANDARD.encode(format!("{head}{tail}"))
        }
    }

    impl App {
        pub(super) async fn make_saml_provider(
            &self,
            idp: &SamlIdp,
            settings: Value,
        ) -> (String, String) {
            let mut base = json!({
                "saml": {"idp_metadata_xml": idp.metadata},
                "claims": {"username": "NameID", "email": "email", "display_name": "name", "groups": "groups"},
            });
            base.as_object_mut()
                .unwrap()
                .extend(settings.as_object().unwrap().clone());
            let res = self
                .admin(
                    Method::POST,
                    "/api/v1/identity-providers",
                    Some(json!({"name": "Acme SAML", "kind": "saml", "settings": base})),
                )
                .await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            let id = res.json["id"].as_str().unwrap().to_string();
            let entity = res.json["sp_entity_id"].as_str().unwrap().to_string();
            (id, entity)
        }

        /// Begin a SAML sign-in: the request id a response must answer,
        /// the relay state and the cookie.
        pub(super) async fn saml_start(&self, provider: &str) -> (String, String, String) {
            let res = self
                .send(
                    Method::GET,
                    &format!("/api/v1/auth/sso/{provider}/start"),
                    &[],
                    None,
                )
                .await;
            assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.text);
            let to = location(&res);
            assert!(to.starts_with(IDP_SSO), "{to}");
            assert!(query_param(&to, "SAMLRequest").is_some(), "{to}");
            let relay = query_param(&to, "RelayState").expect("relay state");
            let cookie = res
                .headers
                .get_all(header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .find(|v| v.starts_with("fcp_saml="))
                .map(|v| v.split(';').next().unwrap().to_string())
                .expect("the SAML state cookie");
            let request_id: String =
                sqlx::query_scalar("SELECT nonce FROM sso_logins WHERE state = $1")
                    .bind(&relay)
                    .fetch_one(self.storage.pool())
                    .await
                    .unwrap();
            (request_id, relay, cookie)
        }

        /// Post a response to the provider's ACS: where the login page is sent.
        pub(super) async fn saml_post(
            &self,
            provider: &str,
            response: &str,
            relay: &str,
            cookie: &str,
        ) -> String {
            let body = format!(
                "SAMLResponse={}&RelayState={}",
                urlencoding::encode(response),
                urlencoding::encode(relay)
            );
            let request = Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v1/auth/saml/{provider}/acs"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, cookie)
                .body(Body::from(body))
                .unwrap();
            let res = self.router.clone().oneshot(request).await.unwrap();
            assert_eq!(res.status(), StatusCode::SEE_OTHER);
            res.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .to_string()
        }
    }
}

#[cfg(feature = "saml")]
#[tokio::test]
async fn a_tenants_people_sign_in_through_its_saml_provider() {
    use saml::{Assertion, SamlIdp};
    let app = require_app!();
    let idp = SamlIdp::new();
    let (provider, entity) = app
        .make_saml_provider(
            &idp,
            json!({"jit": true, "group_roles": [{"group": "it", "role": "tenant_admin"}]}),
        )
        .await;
    assert_eq!(
        entity,
        format!("{SSO_PUBLIC_URL}/api/v1/auth/saml/{provider}")
    );
    let shown = app
        .admin(
            Method::GET,
            &format!("/api/v1/identity-providers/{provider}"),
            None,
        )
        .await;
    assert_eq!(
        shown.json["redirect_uri"],
        format!("{entity}/acs"),
        "{}",
        shown.text
    );
    assert!(shown.json.get("backchannel_logout_uri").is_none());
    let checked = app
        .admin(
            Method::POST,
            &format!("/api/v1/identity-providers/{provider}/check"),
            None,
        )
        .await;
    assert_eq!(checked.json["ok"], true, "{}", checked.text);
    assert_eq!(checked.json["signing_certificates"], 1);

    // The service provider's metadata, for the identity provider's admin.
    let metadata = app
        .send(
            Method::GET,
            &format!("/api/v1/auth/saml/{provider}"),
            &[],
            None,
        )
        .await;
    assert_eq!(metadata.status, StatusCode::OK, "{}", metadata.text);
    assert!(
        metadata.text.contains(&entity) && metadata.text.contains("/acs"),
        "{}",
        metadata.text
    );

    // Signed in, made at first sign-in, an admin by the "it" group.
    let (request_id, relay, cookie) = app.saml_start(&provider).await;
    let mut said = Assertion::new(&request_id, &entity, "rosa@acme.test");
    said.attributes = vec![
        ("email", vec!["rosa@acme.test".into()]),
        ("name", vec!["Rosa Diaz".into()]),
        ("groups", vec!["staff".into(), "it".into()]),
    ];
    let response = said.signed_by(&idp);
    let landing = app.saml_post(&provider, &response, &relay, &cookie).await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(
        signed_in.status,
        StatusCode::OK,
        "{landing} {}",
        signed_in.text
    );
    assert_eq!(signed_in.json["user"]["username"], "rosa");
    assert_eq!(signed_in.json["user"]["role"], "tenant_admin");

    // The same post again: the sign-in it answered is over.
    let again = app.saml_post(&provider, &response, &relay, &cookie).await;
    assert_eq!(query_param(&again, "sso_error").as_deref(), Some("expired"));

    app.finish().await;
}

#[cfg(feature = "saml")]
#[tokio::test]
async fn a_saml_response_is_taken_only_signed_for_us_and_once() {
    use saml::{Assertion, SamlIdp};
    let app = require_app!();
    let idp = SamlIdp::new();
    let (provider, entity) = app.make_saml_provider(&idp, json!({"jit": true})).await;
    let error = |landing: &str| query_param(landing, "sso_error").unwrap_or_default();

    // Each refused response, against a fresh sign-in.
    /// A response made for a sign-in's request id.
    type Make<'a> = Box<dyn Fn(&str) -> String + 'a>;
    let cases: Vec<(&str, Make)> = vec![
        (
            "unsigned",
            Box::new(|req: &str| Assertion::new(req, &entity, "u1").unsigned()),
        ),
        (
            "tampered",
            Box::new(|req: &str| {
                let signed = Assertion::new(req, &entity, "u1").signed_by(&idp);
                let xml = String::from_utf8(
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, signed)
                        .unwrap(),
                )
                .unwrap()
                .replace(">u1<", ">admin<");
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, xml)
            }),
        ),
        (
            "signed by another key",
            Box::new(|req: &str| Assertion::new(req, &entity, "u1").signed_by(&SamlIdp::new())),
        ),
        (
            "for another service provider",
            Box::new(|req: &str| {
                let mut a = Assertion::new(req, &entity, "u1");
                a.sp_entity = "https://elsewhere.test/sp".to_string();
                a.acs = format!("{entity}/acs");
                a.signed_by(&idp)
            }),
        ),
        (
            "a transient subject",
            Box::new(|req: &str| {
                let mut a = Assertion::new(req, &entity, "u1");
                a.name_id_format =
                    "urn:oasis:names:tc:SAML:2.0:nameid-format:transient".to_string();
                a.signed_by(&idp)
            }),
        ),
        (
            "an answer to another request",
            Box::new(|_req: &str| Assertion::new("_other", &entity, "u1").signed_by(&idp)),
        ),
    ];
    for (what, make) in cases {
        let (request_id, relay, cookie) = app.saml_start(&provider).await;
        let landing = app
            .saml_post(&provider, &make(&request_id), &relay, &cookie)
            .await;
        assert_eq!(error(&landing), "bad_assertion", "{what}: {landing}");
    }

    // A post this browser did not begin.
    let (request_id, relay, _) = app.saml_start(&provider).await;
    let (_, _, other_cookie) = app.saml_start(&provider).await;
    let response = Assertion::new(&request_id, &entity, "u1").signed_by(&idp);
    let landing = app
        .saml_post(&provider, &response, &relay, &other_cookie)
        .await;
    assert_eq!(error(&landing), "bad_state");

    // An assertion taken once is not taken again, even answering a new
    // request.
    let (request_id, relay, cookie) = app.saml_start(&provider).await;
    let first = Assertion::new(&request_id, &entity, "u1");
    let landing = app
        .saml_post(&provider, &first.signed_by(&idp), &relay, &cookie)
        .await;
    assert!(query_param(&landing, "sso_code").is_some(), "{landing}");
    let (request_id, relay, cookie) = app.saml_start(&provider).await;
    let mut replayed = Assertion::new(&request_id, &entity, "u1");
    replayed.assertion_id = first.assertion_id.clone();
    let landing = app
        .saml_post(&provider, &replayed.signed_by(&idp), &relay, &cookie)
        .await;
    assert_eq!(error(&landing), "bad_assertion");

    // Metadata that names no signing certificate is no provider: samael
    // would verify nothing against it.
    let refused = app
        .admin(
            Method::POST,
            "/api/v1/identity-providers",
            Some(json!({"name": "x", "kind": "saml", "settings": {"saml": {"idp_metadata_xml": saml::metadata(None)}}})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);

    app.finish().await;
}

#[cfg(feature = "saml")]
#[tokio::test]
async fn fcp_signs_its_saml_requests_and_ends_sessions_the_provider_logs_out() {
    use saml::{Assertion, SamlIdp, IDP_SLO};
    let app = require_app!();
    let idp = SamlIdp::new();
    let (provider, entity) = app.make_saml_provider(&idp, json!({"jit": true})).await;
    let shown = app
        .admin(
            Method::GET,
            &format!("/api/v1/identity-providers/{provider}"),
            None,
        )
        .await;
    assert_eq!(shown.json["sp_key"], true, "{}", shown.text);
    assert_eq!(shown.json["sp_logout_url"], format!("{entity}/slo"));
    let sp_cert = shown.json["settings"]["saml"]["sp_certificate"]
        .as_str()
        .expect("FCP's certificate")
        .to_string();

    // FCP's metadata names its key, its logout and its signed requests.
    let metadata = app
        .send(
            Method::GET,
            &format!("/api/v1/auth/saml/{provider}"),
            &[],
            None,
        )
        .await;
    assert!(metadata.text.contains(&sp_cert), "{}", metadata.text);
    assert!(
        metadata.text.contains(&format!("{entity}/slo")),
        "{}",
        metadata.text
    );
    assert!(
        metadata.text.contains(r#"AuthnRequestsSigned="true""#),
        "{}",
        metadata.text
    );

    // Its requests are signed with that key.
    let res = app
        .send(
            Method::GET,
            &format!("/api/v1/auth/sso/{provider}/start"),
            &[],
            None,
        )
        .await;
    let to = location(&res);
    assert!(
        query_param(&to, "SigAlg").is_some() && query_param(&to, "Signature").is_some(),
        "{to}"
    );
    let verifier = samael::crypto::UrlVerifier::from_x509(&samael::crypto::CertificateDer::from(
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &sp_cert).unwrap(),
    ))
    .unwrap();
    assert!(verifier
        .verify_signed_request_url(&to.parse().unwrap())
        .unwrap());

    // Two sign-ins as two of the provider's sessions; the second's
    // assertion encrypted to FCP's certificate.
    let mut cookies = Vec::new();
    for (session, encrypt) in [("idp-session-1", false), ("idp-session-2", true)] {
        let (request_id, relay, cookie) = app.saml_start(&provider).await;
        let mut said = Assertion::new(&request_id, &entity, "lena@acme.test");
        said.session_index = session.to_string();
        if encrypt {
            said.encrypt_to = Some(sp_cert.clone());
        }
        let landing = app
            .saml_post(&provider, &said.signed_by(&idp), &relay, &cookie)
            .await;
        let signed_in = app.sso_exchange(&landing).await;
        assert_eq!(
            signed_in.status,
            StatusCode::OK,
            "{session}: {landing} {}",
            signed_in.text
        );
        cookies.push(refresh_cookie(&signed_in).unwrap());
    }
    let refreshes = |cookie: String| {
        let app = &app;
        async move {
            app.with_cookie(Method::POST, "/api/v1/auth/refresh", &cookie)
                .await
                .status
        }
    };

    // A forged logout request ends nothing.
    let mut forged = idp.logout_query(&format!("{entity}/slo"), None, Some("idp-session-1"));
    forged = forged.replace("RelayState=rs-1", "RelayState=rs-2");
    let refused = app
        .send(
            Method::GET,
            &format!("/api/v1/auth/saml/{provider}/slo?{forged}"),
            &[],
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    let other_idp = SamlIdp::new();
    let foreign = other_idp.logout_query(&format!("{entity}/slo"), None, Some("idp-session-1"));
    let refused = app
        .send(
            Method::GET,
            &format!("/api/v1/auth/saml/{provider}/slo?{foreign}"),
            &[],
            None,
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert_eq!(refreshes(cookies[0].clone()).await, StatusCode::OK);

    // HTTP-Redirect: the provider's session 1 ends, and FCP answers it,
    // signed.
    let query = idp.logout_query(&format!("{entity}/slo"), None, Some("idp-session-1"));
    let answered = app
        .send(
            Method::GET,
            &format!("/api/v1/auth/saml/{provider}/slo?{query}"),
            &[],
            None,
        )
        .await;
    assert_eq!(answered.status, StatusCode::SEE_OTHER, "{}", answered.text);
    let back = location(&answered);
    assert!(back.starts_with(IDP_SLO), "{back}");
    assert_eq!(query_param(&back, "RelayState").as_deref(), Some("rs-1"));
    assert!(
        verifier
            .verify_signed_response_url(&back.parse().unwrap())
            .unwrap(),
        "{back}"
    );
    assert_eq!(
        refreshes(cookies[0].clone()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(refreshes(cookies[1].clone()).await, StatusCode::OK);

    // HTTP-POST, by the person: every session of theirs ends.
    let body = format!(
        "SAMLRequest={}",
        urlencoding::encode(&idp.logout_post(
            &format!("{entity}/slo"),
            Some("lena@acme.test"),
            None
        ))
    );
    let posted = app
        .send_raw(
            "unused",
            Method::POST,
            &format!("/api/v1/auth/saml/{provider}/slo"),
            "application/x-www-form-urlencoded",
            body.into_bytes(),
        )
        .await;
    assert_eq!(posted.status, StatusCode::SEE_OTHER, "{}", posted.text);
    assert_eq!(
        refreshes(cookies[1].clone()).await,
        StatusCode::UNAUTHORIZED
    );

    // A new key for FCP: its certificate changes.
    let rotated = app
        .admin(
            Method::POST,
            &format!("/api/v1/identity-providers/{provider}/sp-key"),
            None,
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    assert_ne!(
        rotated.json["settings"]["saml"]["sp_certificate"],
        sp_cert.as_str()
    );

    app.finish().await;
}

#[cfg(feature = "saml")]
#[tokio::test]
async fn a_saml_providers_metadata_is_fetched_from_its_url() {
    use saml::SamlIdp;
    let app = require_app!();
    let idp = SamlIdp::new();
    let served = std::sync::Arc::new(std::sync::Mutex::new(idp.metadata.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/metadata", listener.local_addr().unwrap());
    let body = served.clone();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/metadata",
            axum::routing::get(move || {
                let body = body.clone();
                async move { body.lock().unwrap().clone() }
            }),
        );
        let _ = axum::serve(listener, app).await;
    });

    let made = app
        .admin(
            Method::POST,
            "/api/v1/identity-providers",
            Some(json!({"name": "Acme SAML", "kind": "saml", "settings": {"saml": {"idp_metadata_url": url}}})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert!(made.json["settings"]["saml"]["idp_metadata_xml"]
        .as_str()
        .unwrap()
        .contains("EntityDescriptor"));
    let provider = made.json["id"].as_str().unwrap().to_string();
    let checked = app
        .admin(
            Method::POST,
            &format!("/api/v1/identity-providers/{provider}/check"),
            None,
        )
        .await;
    assert_eq!(checked.json["ok"], true, "{}", checked.text);
    assert_eq!(checked.json["idp_logout_url"], saml::IDP_SLO);

    // The provider rotates its certificate; a refresh picks it up.
    let rotated = SamlIdp::new();
    *served.lock().unwrap() = rotated.metadata.clone();
    let refreshed = app
        .admin(
            Method::POST,
            &format!("/api/v1/identity-providers/{provider}/refresh-metadata"),
            None,
        )
        .await;
    assert_eq!(refreshed.status, StatusCode::OK, "{}", refreshed.text);
    assert_eq!(
        refreshed.json["settings"]["saml"]["idp_metadata_xml"],
        rotated.metadata.as_str()
    );

    server.abort();
    app.finish().await;
}

// ---------------------------------------------------------------------------
// A user's lifecycle (docs/IDENTITY_AND_DEVICES.md phase 5a)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_suspended_user_is_refused_everywhere_and_their_phones_go_quiet() {
    let app = require_app!();
    app.create_user_with_role("sam", "end_user").await;
    app.create_user_with_role("kay", "end_user").await;
    let realm = REALM;
    app.seed_registration(
        app.tenant_a,
        &format!("sip:sam@{realm}"),
        "sip:sam@10.0.0.5:5060",
    )
    .await;
    app.seed_registration(
        app.tenant_a,
        &format!("sip:kay@{realm}"),
        "sip:kay@10.0.0.6:5060",
    )
    .await;
    let signed_in = app.login("sam", PASSWORD).await;
    let cookie = refresh_cookie(&signed_in).unwrap();
    let session = signed_in.json["token"].as_str().unwrap().to_string();
    let token = app
        .send_as(
            &session,
            Method::POST,
            "/api/v1/me/api-tokens",
            Some(json!({"name": "sam's script", "permissions": []})),
        )
        .await;
    assert_eq!(token.status, StatusCode::CREATED, "{}", token.text);
    let token = token.json["secret"].as_str().unwrap().to_string();

    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .unwrap();
    let suspended = app
        .admin(
            Method::PUT,
            "/api/v1/users/sam",
            Some(json!({"account_status": "suspended"})),
        )
        .await;
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.text);
    assert_eq!(
        suspended.json["enabled"], false,
        "enabled follows the status"
    );

    // Signed out, refused at sign-in, their token refused.
    assert_eq!(
        app.with_cookie(Method::POST, "/api/v1/auth/refresh", &cookie)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.login("sam", PASSWORD).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send_as(&token, Method::GET, "/api/v1/me/profile", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    // Their phones dropped; another's kept.
    use fcp_core::storage::RegistrationStore;
    let theirs = app
        .storage
        .get_registrations_in_tenant(&app.tenant_a, &format!("sip:sam@{realm}"))
        .await
        .unwrap();
    assert!(theirs.is_empty(), "{theirs:?}");
    let kays = app
        .storage
        .get_registrations_in_tenant(&app.tenant_a, &format!("sip:kay@{realm}"))
        .await
        .unwrap();
    assert_eq!(kays.len(), 1);

    // Announced: the registration gone, the user disabled, what changed.
    let (mut unregistered, mut disabled, mut changed) = (false, false, Vec::new());
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            match event {
                PlatformEvent::Registration(fcp_events::RegistrationEvent::Unregistered {
                    aor,
                    ..
                }) if aor.contains("sam@") => unregistered = true,
                PlatformEvent::User(fcp_events::UserEvent::Disabled { username, .. })
                    if username == "sam" =>
                {
                    disabled = true
                }
                PlatformEvent::User(fcp_events::UserEvent::Updated {
                    username,
                    changed_fields,
                    ..
                }) if username == "sam" => changed = changed_fields,
                _ => {}
            }
            if unregistered && disabled && !changed.is_empty() {
                break;
            }
        }
    })
    .await;
    assert!(
        unregistered && disabled,
        "unregistered {unregistered}, disabled {disabled}"
    );
    assert!(
        changed.contains(&"account_status".to_string()) && changed.contains(&"enabled".to_string()),
        "{changed:?}"
    );

    // Resumed the old way, `enabled: true`: active, and signing in again.
    let resumed = app
        .admin(
            Method::PUT,
            "/api/v1/users/sam",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(resumed.json["account_status"], "active", "{}", resumed.text);
    assert_eq!(app.login("sam", PASSWORD).await.status, StatusCode::OK);

    // A status that is none is refused, where it used to become active;
    // the UI's old "disabled" is a suspension.
    let bogus = app
        .admin(
            Method::PUT,
            "/api/v1/users/kay",
            Some(json!({"account_status": "bogus"})),
        )
        .await;
    assert_eq!(bogus.status, StatusCode::BAD_REQUEST, "{}", bogus.text);
    let disabled = app
        .admin(
            Method::PUT,
            "/api/v1/users/kay",
            Some(json!({"account_status": "disabled"})),
        )
        .await;
    assert_eq!(
        disabled.json["account_status"], "suspended",
        "{}",
        disabled.text
    );

    app.finish().await;
}

#[tokio::test]
async fn a_tenants_users_are_listed_filtered_and_capped() {
    let app = require_app!();
    for name in ["ann", "andy", "bob"] {
        app.create_user_with_role(name, "end_user").await;
    }
    app.admin(
        Method::PUT,
        "/api/v1/users/bob",
        Some(json!({"account_status": "suspended"})),
    )
    .await;

    let names = |res: &Response| -> Vec<String> {
        res.json
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["username"].as_str().unwrap().to_string())
            .collect()
    };
    let found = app.admin(Method::GET, "/api/v1/users?q=AN", None).await;
    assert_eq!(names(&found), ["andy", "ann"], "{}", found.text);
    let suspended = app
        .admin(Method::GET, "/api/v1/users?status=suspended", None)
        .await;
    assert_eq!(names(&suspended), ["bob"], "{}", suspended.text);
    let wrong = app
        .admin(Method::GET, "/api/v1/users?status=gone", None)
        .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);

    // The tenant's max_users holds.
    let all = app.admin(Method::GET, "/api/v1/users", None).await;
    let count = all.body["total"].as_u64().unwrap();
    app.admin(
        Method::PUT,
        &format!("/api/v1/tenants/{}", app.tenant_a),
        Some(json!({"max_users": count})),
    )
    .await;
    let over = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "one-too-many", "password": PASSWORD})),
        )
        .await;
    assert_eq!(over.status, StatusCode::CONFLICT, "{}", over.text);

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Deprovisioning and the purge (docs/IDENTITY_AND_DEVICES.md phase 5b)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_deleted_user_waits_out_the_grace_period_and_is_purged_from_every_store() {
    use fcp_core::storage::VoicemailStore;
    let app = require_app!();
    let pool = app.storage.pool().clone();
    let tenant = app.tenant_a.to_string();
    let dee_id = app.create_user_with_role("dee", "end_user").await;
    app.create_user_with_role("boss", "end_user").await;
    app.create_user_with_role("owner", "end_user").await;
    app.create_user_with_role("ta", "tenant_admin").await;
    let set = app
        .admin(
            Method::PUT,
            "/api/v1/users/dee",
            Some(json!({"extension": "2001"})),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);

    // The tenant's offboarding: checked, then set.
    for bad in [
        json!({"grace_days": 99_999}),
        json!({"grace_days": 30, "meetings_owner": "nobody"}),
    ] {
        let refused = app
            .admin(
                Method::PUT,
                &format!("/api/v1/tenants/{}", app.tenant_a),
                Some(json!({ "offboarding": bad })),
            )
            .await;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    }
    let set = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({ "offboarding": {"grace_days": 30, "meetings_owner": "owner"} })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    assert_eq!(set.json["offboarding"]["grace_days"], 30, "{}", set.text);

    // Everything Dee has: a phone, a line on a number, a mailbox with a
    // message, an extension, calling settings, an assistant's place, a
    // token, a registration, a declared location, a queue seat, a meeting.
    let (device, _) = app
        .create_phone("dee's phone", "00:15:65:00:55:01", Some(&dee_id))
        .await;
    let number = app
        .admin(
            Method::POST,
            "/api/v1/phone-numbers",
            Some(json!({ "e164": "+15550005501", "tenant_id": app.tenant_a })),
        )
        .await;
    assert_eq!(number.status, StatusCode::CREATED, "{}", number.text);
    let number_id = number.json["id"].as_str().unwrap().to_string();
    let line = app
        .admin(
            Method::POST,
            "/api/v1/line-bindings",
            Some(json!({
                "phone_number_id": number_id,
                "owner_user_id": dee_id,
                "binding_type": "user",
                "tenant_id": app.tenant_a,
            })),
        )
        .await;
    assert_eq!(line.status, StatusCode::CREATED, "{}", line.text);
    let line_id: fcp_core::LineBindingId = line.json["id"].as_str().unwrap().parse().unwrap();
    let mailbox = fcp_core::VoicemailMailbox::new(
        app.tenant_a,
        dee_id.parse().unwrap(),
        line_id,
        "dee".to_string(),
        "1234".to_string(),
    );
    let mailbox_id = mailbox.id.clone();
    app.storage.put_mailbox(mailbox).await.unwrap();
    let message = fcp_core::VoicemailMessage::new(
        mailbox_id.clone(),
        "sip:caller@example.com".to_string(),
        12,
        std::path::PathBuf::from("/var/lib/fcp/voicemail/dee-1.wav"),
    );
    let message_id = message.id.clone();
    app.storage.put_message(message).await.unwrap();
    sqlx::query(
        "INSERT INTO extensions (id, number, target_type, target_id, tenant_id)
         VALUES ('ext-dee', '2001', 'user', $1, $2)",
    )
    .bind(&dee_id)
    .bind(&tenant)
    .execute(&pool)
    .await
    .unwrap();
    let set = app
        .admin(
            Method::PUT,
            "/api/v1/users/dee/calling",
            Some(json!({ "dnd": true })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let set = app
        .admin(
            Method::PUT,
            "/api/v1/users/boss/calling",
            Some(json!({ "executive": { "enabled": true, "assistants": ["dee"] } })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let session = app.login("dee", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let token = app
        .send_as(
            &session,
            Method::POST,
            "/api/v1/me/api-tokens",
            Some(json!({"name": "dee's script", "permissions": []})),
        )
        .await;
    assert_eq!(token.status, StatusCode::CREATED, "{}", token.text);
    let token = token.json["secret"].as_str().unwrap().to_string();
    app.seed_registration(
        app.tenant_a,
        &format!("sip:dee@{REALM}"),
        "sip:dee@10.0.0.7:5060",
    )
    .await;
    for statement in [
        "INSERT INTO declared_locations (tenant_id, user_id, address, expires_at)
         VALUES ($1, $2, '{}'::jsonb, now() + interval '1 day')",
        "INSERT INTO queue_agents (tenant_id, agent_id, extension) VALUES ($1, 'dee', '2001')",
        "INSERT INTO conference_meetings (id, tenant_id, room_number, name, owner_id)
         VALUES ('meeting-dee', $1, '9001', 'Dee''s room', 'dee')",
    ] {
        let query = sqlx::query(statement).bind(&tenant);
        let query = if statement.contains("$2") {
            query.bind(&dee_id)
        } else {
            query
        };
        query.execute(&pool).await.unwrap();
    }

    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .unwrap();

    // Deleted: deprovisioned, refused everywhere, and changed by nothing
    // but a restore.
    let deleted = app.admin(Method::DELETE, "/api/v1/users/dee", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    let dee = app.admin(Method::GET, "/api/v1/users/dee", None).await;
    assert_eq!(dee.json["account_status"], "deprovisioned", "{}", dee.text);
    assert_eq!(dee.json["enabled"], false);
    assert!(dee.json["deprovisioned_at"].is_string(), "{}", dee.text);
    assert_eq!(
        app.login("dee", PASSWORD).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send_as(&token, Method::GET, "/api/v1/me/profile", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let edited = app
        .admin(
            Method::PUT,
            "/api/v1/users/dee",
            Some(json!({"display_name": "Dee"})),
        )
        .await;
    assert_eq!(edited.status, StatusCode::CONFLICT, "{}", edited.text);
    let not_settable = app
        .admin(
            Method::PUT,
            "/api/v1/users/boss",
            Some(json!({"account_status": "deprovisioned"})),
        )
        .await;
    assert_eq!(not_settable.status, StatusCode::BAD_REQUEST);
    let listed = app
        .admin(Method::GET, "/api/v1/users?status=deprovisioned", None)
        .await;
    assert_eq!(listed.json.as_array().unwrap().len(), 1, "{}", listed.text);
    let purge_after = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if let PlatformEvent::User(fcp_events::UserEvent::Deprovisioned {
                username,
                purge_after,
                ..
            }) = event
            {
                if username == "dee" {
                    return purge_after;
                }
            }
        }
        unreachable!()
    })
    .await
    .expect("user.deprovisioned");
    let in_days = purge_after
        .duration_since(std::time::SystemTime::now())
        .unwrap()
        .as_secs()
        / 86_400;
    assert_eq!(in_days, 29, "purged thirty days on");

    // Restored: as they were, and signing in.
    let restored = app
        .admin(Method::POST, "/api/v1/users/dee/restore", None)
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.text);
    assert_eq!(restored.json["account_status"], "active");
    assert!(restored.json["deprovisioned_at"].is_null());
    assert_eq!(app.login("dee", PASSWORD).await.status, StatusCode::OK);
    let again = app
        .admin(Method::POST, "/api/v1/users/dee/restore", None)
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);

    // Deleted again. A tenant admin cannot purge early; the sweep does not
    // before the grace period ends, nor while an erasure for Dee is open.
    app.admin(Method::DELETE, "/api/v1/users/dee", None).await;
    let ta = app.login("ta", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let early = app
        .send_as(&ta, Method::POST, "/api/v1/users/dee/purge", None)
        .await;
    assert_eq!(early.status, StatusCode::FORBIDDEN, "{}", early.text);
    let now = std::time::SystemTime::now();
    let swept = fcp_api_core::offboarding::run_sweep(&app.state, now, "test").await;
    assert_eq!(swept.purged, 0, "{swept:?}");
    sqlx::query(
        "INSERT INTO subject_requests
             (id, tenant_id, kind, subjects, status, received_at, due_at, requested_by)
         VALUES ('sr-dee', $1, 'erasure', '[{\"type\": \"user\", \"username\": \"dee\"}]'::jsonb,
                 'preview', now(), now() + interval '30 days', 'admin')",
    )
    .bind(&tenant)
    .execute(&pool)
    .await
    .unwrap();
    let later = now + Duration::from_secs(31 * 86_400);
    let swept = fcp_api_core::offboarding::run_sweep(&app.state, later, "test").await;
    assert_eq!(
        (swept.purged, swept.waiting_for_erasure),
        (0, 1),
        "{swept:?}"
    );
    sqlx::query("UPDATE subject_requests SET status = 'completed' WHERE id = 'sr-dee'")
        .execute(&pool)
        .await
        .unwrap();

    // The grace period over: purged.
    let swept = fcp_api_core::offboarding::run_sweep(&app.state, later, "test").await;
    assert_eq!(swept.purged, 1, "{swept:?}");
    assert!(swept.errors.is_empty(), "{swept:?}");
    assert_eq!(
        app.admin(Method::GET, "/api/v1/users/dee", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let count = |sql: &'static str| {
        let pool = pool.clone();
        let tenant = tenant.clone();
        let dee_id = dee_id.clone();
        async move {
            let query = sqlx::query_scalar::<_, i64>(sql).bind(tenant);
            let query = if sql.contains("$2") {
                query.bind(dee_id)
            } else {
                query
            };
            query.fetch_one(&pool).await.unwrap()
        }
    };
    // Kept by the tenant: the phone, unassigned; the number, released.
    assert_eq!(
        count("SELECT count(*) FROM devices WHERE tenant_id = $1 AND owner_user_id IS NULL AND name = 'dee''s phone'").await,
        1
    );
    let _ = &device;
    assert_eq!(
        count("SELECT count(*) FROM phone_numbers WHERE tenant_id = $1 AND e164 = '+15550005501'")
            .await,
        1
    );
    for (what, sql) in [
        (
            "line",
            "SELECT count(*) FROM line_bindings WHERE tenant_id = $1 AND owner_user_id = $2",
        ),
        (
            "extension",
            "SELECT count(*) FROM extensions WHERE tenant_id = $1 AND target_id = $2",
        ),
        (
            "settings",
            "SELECT count(*) FROM user_calling_settings WHERE tenant_id = $1 AND user_id = $2",
        ),
        (
            "tokens",
            "SELECT count(*) FROM api_tokens WHERE tenant_id = $1 AND user_id = $2",
        ),
        (
            "location",
            "SELECT count(*) FROM declared_locations WHERE tenant_id = $1 AND user_id = $2",
        ),
        (
            "queue",
            "SELECT count(*) FROM queue_agents WHERE tenant_id = $1 AND agent_id = 'dee'",
        ),
        (
            "registrations",
            "SELECT count(*) FROM registrations WHERE tenant_id = $1 AND aor LIKE '%dee@%'",
        ),
        (
            "sessions",
            "SELECT count(*) FROM login_sessions WHERE tenant_id = $1 AND user_id = $2",
        ),
    ] {
        assert_eq!(count(sql).await, 0, "{what} left behind");
    }
    // Forgotten by the executive; the meeting is the owner's.
    let boss = app
        .admin(Method::GET, "/api/v1/users/boss/calling", None)
        .await;
    assert_eq!(
        boss.json["executive"]["assistants"],
        json!([]),
        "{}",
        boss.text
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM conference_meetings WHERE tenant_id = $1 AND owner_id = 'owner'"
        )
        .await,
        1
    );
    // The mailbox kept, closed and off its line, while it holds a message.
    let kept = app.storage.get_mailbox(&mailbox_id).await.unwrap().unwrap();
    assert_eq!(kept.line_binding_id, None);
    assert!(!kept.enabled);
    assert!(
        kept.extension.starts_with("offboarded-"),
        "{}",
        kept.extension
    );
    // Logged, and announced.
    assert_eq!(
        count("SELECT count(*) FROM deletion_log WHERE tenant_id = $1 AND class = 'user' AND item = $2 AND cause = 'offboarding'")
            .await,
        1
    );
    let deleted = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if let PlatformEvent::User(fcp_events::UserEvent::Deleted { username, .. }) = event {
                if username == "dee" {
                    return true;
                }
            }
        }
        false
    })
    .await;
    assert_eq!(deleted, Ok(true), "user.deleted");

    // Retention empties the mailbox; the sweep then removes it.
    app.storage.delete_message(&message_id).await.unwrap();
    let swept = fcp_api_core::offboarding::run_sweep(&app.state, later, "test").await;
    assert_eq!(swept.mailboxes_removed, 1, "{swept:?}");
    assert!(app
        .storage
        .get_mailbox(&mailbox_id)
        .await
        .unwrap()
        .is_none());

    // The username is free again.
    app.create_user_with_role("dee", "end_user").await;
    let purged_again = app
        .dyn_storage
        .offboarding_store()
        .unwrap()
        .deprovisioned_users(&app.tenant_a, later)
        .await
        .unwrap();
    assert!(purged_again.is_empty());

    app.finish().await;
}

#[tokio::test]
async fn a_tenant_without_a_grace_period_purges_at_once_and_a_platform_admin_may_early() {
    let app = require_app!();
    app.create_user_with_role("zed", "end_user").await;
    app.create_user_with_role("yan", "end_user").await;

    // A platform admin purges before the grace period ends; only someone
    // deleted.
    let not_deleted = app
        .admin(Method::POST, "/api/v1/users/yan/purge", None)
        .await;
    assert_eq!(
        not_deleted.status,
        StatusCode::CONFLICT,
        "{}",
        not_deleted.text
    );
    app.admin(Method::DELETE, "/api/v1/users/yan", None).await;
    let purged = app
        .admin(Method::POST, "/api/v1/users/yan/purge", None)
        .await;
    assert_eq!(purged.status, StatusCode::OK, "{}", purged.text);
    assert_eq!(purged.json["username"], "yan");
    assert!(purged.json["report"].is_object(), "{}", purged.text);

    // A grace period of 0: a delete is a purge.
    let set = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({ "offboarding": {"grace_days": 0} })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let deleted = app.admin(Method::DELETE, "/api/v1/users/zed", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text);
    assert_eq!(
        app.admin(Method::GET, "/api/v1/users/zed", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Groups (docs/IDENTITY_AND_DEVICES.md phase 5c)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_groups_role_is_its_members_too_and_roles_add_up() {
    let app = require_app!();
    app.create_user_with_role("gina", "end_user").await;
    app.create_user_with_role("rita", "read_only").await;
    let gina = app.login("gina", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let rita = app.login("rita", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let users = |token: String| {
        let app = &app;
        async move {
            app.send_as(&token, Method::GET, "/api/v1/users", None)
                .await
                .status
        }
    };
    assert_eq!(users(gina.clone()).await, StatusCode::FORBIDDEN);

    // Only the tenant's administrator makes groups, and no group makes a
    // platform admin.
    let refused = app
        .send_as(
            &rita,
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "Sneaky", "role": "tenant_admin"})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    for (body, status) in [
        (
            json!({"name": "Root", "role": "super_admin"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"name": "Dept", "role": "department_admin"}),
            StatusCode::BAD_REQUEST,
        ),
        (json!({"name": "  "}), StatusCode::BAD_REQUEST),
    ] {
        let res = app.admin(Method::POST, "/api/v1/groups", Some(body)).await;
        assert_eq!(res.status, status, "{}", res.text);
    }
    let helpdesk = app
        .admin(
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "Helpdesk", "role": "user_admin", "description": "first line"})),
        )
        .await;
    assert_eq!(helpdesk.status, StatusCode::CREATED, "{}", helpdesk.text);
    let helpdesk_id = helpdesk.json["id"].as_str().unwrap().to_string();
    assert!(helpdesk_id.starts_with("grp-"), "{helpdesk_id}");
    let twin = app
        .admin(
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "HELPDESK"})),
        )
        .await;
    assert_eq!(twin.status, StatusCode::CONFLICT, "{}", twin.text);
    let telecom = app
        .admin(
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "Telecom", "role": "telecom_admin"})),
        )
        .await;
    let telecom_id = telecom.json["id"].as_str().unwrap().to_string();

    // In the group, Gina holds its role at once; out of it, at once not.
    let added = app
        .admin(
            Method::POST,
            &format!("/api/v1/groups/{helpdesk_id}/members"),
            Some(json!({"username": "gina"})),
        )
        .await;
    assert_eq!(added.status, StatusCode::NO_CONTENT, "{}", added.text);
    assert_eq!(users(gina.clone()).await, StatusCode::OK);
    let me = app
        .send_as(&gina, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.json["role"], "user_admin", "{}", me.text);
    let members = app
        .admin(
            Method::GET,
            &format!("/api/v1/groups/{helpdesk_id}/members"),
            None,
        )
        .await;
    assert_eq!(members.json[0]["username"], "gina", "{}", members.text);
    let listed = app.admin(Method::GET, "/api/v1/groups", None).await;
    let helpdesk_listed = listed
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["name"] == "Helpdesk")
        .cloned()
        .unwrap();
    assert_eq!(helpdesk_listed["member_count"], 1, "{}", listed.text);
    assert_eq!(helpdesk_listed["role"], "user_admin");

    // Two groups: what each role holds, together. A user admin manages
    // users and a telecom admin queues; "the highest" would lose one.
    app.admin(
        Method::POST,
        &format!("/api/v1/groups/{telecom_id}/members"),
        Some(json!({"username": "gina"})),
    )
    .await;
    let made = app
        .send_as(
            &gina,
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "newbie", "password": PASSWORD})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let queues = app
        .send_as(&gina, Method::GET, "/api/v1/queue-configs", None)
        .await;
    assert_eq!(queues.status, StatusCode::OK, "{}", queues.text);
    let theirs = app
        .admin(Method::GET, "/api/v1/users/gina/groups", None)
        .await;
    assert_eq!(theirs.json.as_array().unwrap().len(), 2, "{}", theirs.text);

    let removed = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/groups/{helpdesk_id}/members/gina"),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let refused = app
        .send_as(
            &gina,
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "newbie2", "password": PASSWORD})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);

    // A queue takes its agents from a group; the group gone, from nobody.
    let queue = app
        .admin(
            Method::POST,
            "/api/v1/queue-configs",
            Some(json!({"name": "noc", "extension": "7101", "agent_group_id": telecom_id})),
        )
        .await;
    assert_eq!(queue.status, StatusCode::CREATED, "{}", queue.text);
    assert_eq!(queue.json["agent_group_id"], telecom_id.as_str());
    let bogus = app
        .admin(
            Method::POST,
            "/api/v1/queue-configs",
            Some(json!({"name": "bad", "extension": "7102", "agent_group_id": "grp-00000000-0000-0000-0000-000000000000"})),
        )
        .await;
    assert_eq!(bogus.status, StatusCode::BAD_REQUEST, "{}", bogus.text);
    let gone = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/groups/{telecom_id}"),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    let queue_id = queue.json["id"].as_str().unwrap();
    let queue = app
        .admin(
            Method::GET,
            &format!("/api/v1/queue-configs/{queue_id}"),
            None,
        )
        .await;
    assert!(queue.json["agent_group_id"].is_null(), "{}", queue.text);
    assert_eq!(users(gina.clone()).await, StatusCode::FORBIDDEN);

    app.finish().await;
}

#[tokio::test]
async fn groups_that_follow_the_identity_provider_are_set_at_every_sign_in() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    app.admin(
        Method::PUT,
        &format!("/api/v1/tenants/{}", app.tenant_a),
        Some(json!({"login_domains": ["acme.test"]})),
    )
    .await;
    let provider = app.make_provider(&idp, json!({"jit": true})).await;
    let following = app
        .admin(
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "Ops", "role": "telecom_admin", "idp_group": "ops"})),
        )
        .await;
    let following_id = following.json["id"].as_str().unwrap().to_string();
    let own = app
        .admin(
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "Floor 3"})),
        )
        .await;
    let own_id = own.json["id"].as_str().unwrap().to_string();
    let count = |id: String| {
        let app = &app;
        async move {
            app.admin(Method::GET, &format!("/api/v1/groups/{id}"), None)
                .await
                .json["member_count"]
                .as_u64()
                .unwrap()
        }
    };

    // Sam signs in in the provider's "ops": he is in Ops, and holds its role.
    let sam = json!({
        "sub": "sam-1", "email": "sam@acme.test", "email_verified": true,
        "preferred_username": "sam@acme.test", "groups": ["ops", "staff"],
    });
    let landing = app.sso_landing(&idp, &provider, sam.clone()).await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(signed_in.status, StatusCode::OK, "{}", signed_in.text);
    assert_eq!(
        signed_in.json["user"]["role"], "telecom_admin",
        "{}",
        signed_in.text
    );
    assert_eq!(count(following_id.clone()).await, 1);

    // An admin puts him on Floor 3 too: the tenant's own group, which no
    // sign-in touches.
    app.admin(
        Method::POST,
        &format!("/api/v1/groups/{own_id}/members"),
        Some(json!({"username": "sam"})),
    )
    .await;

    // Out of "ops" at the provider: out of Ops at the next sign-in.
    let mut moved = sam.clone();
    moved["groups"] = json!(["staff"]);
    let landing = app.sso_landing(&idp, &provider, moved).await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(
        signed_in.json["user"]["role"], "end_user",
        "{}",
        signed_in.text
    );
    assert_eq!(count(following_id).await, 0);
    assert_eq!(count(own_id).await, 1);

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Renaming a user (docs/IDENTITY_AND_DEVICES.md phase 5d)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_renamed_user_keeps_their_sip_name_their_sessions_and_what_they_had() {
    use fcp_core::storage::password::compute_ha1;
    use fcp_core::CredentialStore;
    let app = require_app!();
    let pool = app.storage.pool().clone();
    let tenant = app.tenant_a.to_string();
    app.create_user_with_role("ann", "end_user").await;
    app.create_user_with_role("ben", "end_user").await;
    let session = app.login("ann", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let group = app
        .admin(
            Method::POST,
            "/api/v1/groups",
            Some(json!({"name": "Sales"})),
        )
        .await;
    let group_id = group.json["id"].as_str().unwrap().to_string();
    app.admin(
        Method::POST,
        &format!("/api/v1/groups/{group_id}/members"),
        Some(json!({"username": "ann"})),
    )
    .await;
    sqlx::query(
        "INSERT INTO conference_meetings (id, tenant_id, room_number, name, owner_id)
         VALUES ('meeting-ann', $1, '9101', 'Ann''s room', 'ann')",
    )
    .bind(&tenant)
    .execute(&pool)
    .await
    .unwrap();
    let mut events = fcp_events::EventSubscriber::subscribe(
        app.event_bus.as_ref(),
        fcp_events::EventFilter::new(),
    )
    .await
    .unwrap();

    // Ann marries: her login changes; her phones' name does not.
    let renamed = app
        .admin(
            Method::PUT,
            "/api/v1/users/ann",
            Some(json!({"username": "ann.lee", "display_name": "Ann Lee"})),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text);
    assert_eq!(renamed.json["username"], "ann.lee");
    assert_eq!(renamed.json["sip_username"], "ann");
    assert_eq!(renamed.json["display_name"], "Ann Lee");
    assert_eq!(
        app.admin(Method::GET, "/api/v1/users/ann", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let renamed_event = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if let PlatformEvent::User(fcp_events::UserEvent::Renamed {
                old_username,
                username,
                ..
            }) = event
            {
                return (old_username, username);
            }
        }
        unreachable!()
    })
    .await
    .expect("user.renamed");
    assert_eq!(renamed_event, ("ann".to_string(), "ann.lee".to_string()));

    // She signs in by her new name; her session from before is hers still.
    assert_eq!(app.login("ann.lee", PASSWORD).await.status, StatusCode::OK);
    assert_ne!(app.login("ann", PASSWORD).await.status, StatusCode::OK);
    let me = app
        .send_as(&session, Method::GET, "/api/v1/auth/me", None)
        .await;
    assert_eq!(me.json["username"], "ann.lee", "{}", me.text);

    // What she had is hers: her group, her meeting.
    let groups = app
        .admin(Method::GET, "/api/v1/users/ann.lee/groups", None)
        .await;
    assert_eq!(groups.json[0]["name"], "Sales", "{}", groups.text);
    let owner: String =
        sqlx::query_scalar("SELECT owner_id FROM conference_meetings WHERE id = 'meeting-ann'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(owner, "ann.lee");

    // Her phones' name is hers: nobody may take "ann", as a username or
    // by a rename; a new password's digest is over it.
    let taken = app
        .admin(
            Method::POST,
            "/api/v1/users",
            Some(json!({"username": "ann", "password": PASSWORD})),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);
    let taken = app
        .admin(
            Method::PUT,
            "/api/v1/users/ben",
            Some(json!({"username": "ann"})),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{}", taken.text);
    let id_shaped = app
        .admin(
            Method::PUT,
            "/api/v1/users/ben",
            Some(json!({"username": format!("{}", fcp_core::UserId::new())})),
        )
        .await;
    assert_eq!(
        id_shaped.status,
        StatusCode::BAD_REQUEST,
        "{}",
        id_shaped.text
    );
    let changed = app
        .admin(
            Method::PUT,
            "/api/v1/users/ann.lee/password",
            Some(json!({"password": "Another-Passw0rd!"})),
        )
        .await;
    assert!(changed.status.is_success(), "{}", changed.text);
    let stored = app
        .storage
        .get_user_in_tenant("ann.lee", &app.tenant_a)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.sip_ha1_hash.as_deref(),
        Some(compute_ha1("ann", REALM, "Another-Passw0rd!").as_str()),
        "the digest is over the SIP name"
    );
    assert_eq!(
        app.storage
            .get_user_by_sip_name("ann", &app.tenant_a)
            .await
            .unwrap()
            .map(|u| u.username),
        Some("ann.lee".to_string())
    );

    // Back to her old name: hers to take.
    let back = app
        .admin(
            Method::PUT,
            "/api/v1/users/ann.lee",
            Some(json!({"username": "ann"})),
        )
        .await;
    assert_eq!(back.status, StatusCode::OK, "{}", back.text);
    assert_eq!(back.json["username"], "ann");
    assert_eq!(back.json["sip_username"], "ann");

    app.finish().await;
}

// ---------------------------------------------------------------------------
// SCIM 2.0 (docs/IDENTITY_AND_DEVICES.md phase 6, docs/SCIM.md)
// ---------------------------------------------------------------------------

impl App {
    /// An API token holding `scim_provision`, as a tenant admin gives a
    /// provisioner.
    async fn scim_token(&self) -> String {
        let made = self
            .admin(
                Method::POST,
                "/api/v1/api-tokens",
                Some(json!({"name": "provisioner", "permissions": ["scim_provision"]})),
            )
            .await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
        made.json["secret"].as_str().unwrap().to_string()
    }
}

fn scim_user_id(res: &Response) -> String {
    res.json["id"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn entra_id_provisions_updates_and_removes_a_user() {
    let app = require_app!();
    let token = app.scim_token().await;
    let scim = |method: Method, path: String, body: Option<Value>| {
        let (app, token) = (&app, token.clone());
        async move { app.send_as(&token, method, &path, body).await }
    };

    // Discovery.
    let config = scim(Method::GET, "/scim/v2/ServiceProviderConfig".into(), None).await;
    assert_eq!(config.status, StatusCode::OK, "{}", config.text);
    assert_eq!(config.json["patch"]["supported"], true);
    assert_eq!(
        config.headers.get(header::CONTENT_TYPE).unwrap(),
        "application/scim+json"
    );
    let types = scim(Method::GET, "/scim/v2/ResourceTypes".into(), None).await;
    assert_eq!(
        types.json["Resources"][0]["endpoint"], "/Users",
        "{}",
        types.text
    );

    // Entra ID asks first whether the user is there, then makes them.
    let user_name = "Test_User_ab6490ee-1e48-479e-a20b-2d77186b5dd1";
    let none = scim(
        Method::GET,
        format!("/scim/v2/Users?filter=userName%20eq%20%22{user_name}%22"),
        None,
    )
    .await;
    assert_eq!(none.json["totalResults"], 0, "{}", none.text);
    let made = scim(
        Method::POST,
        "/scim/v2/Users".into(),
        Some(json!({
            "schemas": [
                "urn:ietf:params:scim:schemas:core:2.0:User",
                "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"
            ],
            "externalId": "0a21f0f2-8d2a-4f8e-bf98-7363c4aed4ef",
            "userName": user_name,
            "active": true,
            "emails": [{"primary": true, "type": "work",
                        "value": "Test_User_fd0ea19b@testuser.com"}],
            "meta": {"resourceType": "User"},
            "name": {"formatted": "givenName familyName",
                     "familyName": "familyName", "givenName": "givenName"},
            "roles": []
        })),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = scim_user_id(&made);
    assert!(id.starts_with("user-"), "{id}");
    assert_eq!(made.json["active"], true);
    assert!(made.headers.get(header::ETAG).is_some());
    assert!(made.headers.get(header::LOCATION).is_some());
    let again = scim(
        Method::POST,
        "/scim/v2/Users".into(),
        Some(json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"], "userName": user_name})),
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text);
    assert_eq!(again.json["scimType"], "uniqueness");

    // The user is FCP's: an end user who may not sign in with a password.
    let fcp = app
        .admin(Method::GET, &format!("/api/v1/users/{user_name}"), None)
        .await;
    assert_eq!(fcp.json["role"], "end_user", "{}", fcp.text);
    assert_eq!(fcp.json["email"], "Test_User_fd0ea19b@testuser.com");
    assert_eq!(fcp.json["display_name"], "givenName familyName");

    // Found by externalId; PATCHes in Entra ID's shape.
    let by_external = scim(
        Method::GET,
        "/scim/v2/Users?filter=externalId%20eq%20%220a21f0f2-8d2a-4f8e-bf98-7363c4aed4ef%22".into(),
        None,
    )
    .await;
    assert_eq!(by_external.json["Resources"][0]["id"], id.as_str());
    let patched = scim(
        Method::PATCH,
        format!("/scim/v2/Users/{id}"),
        Some(json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [
                {"op": "Replace", "path": "emails[type eq \"work\"].value", "value": "updatedEmail@microsoft.com"},
                {"op": "Replace", "path": "name.familyName", "value": "updatedFamilyName"},
                {"op": "Add", "path": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department", "value": "Sales"},
                {"op": "Add", "path": "title", "value": "Engineer"}
            ]
        })),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.text);
    assert_eq!(
        patched.json["emails"][0]["value"],
        "updatedEmail@microsoft.com"
    );
    assert_eq!(
        patched.json["name"]["formatted"],
        "givenName updatedFamilyName"
    );
    assert_eq!(
        patched.json["urn:ietf:params:scim:schemas:extension:enterprise:2.0:User"]["department"],
        "Sales"
    );

    // A stale version is refused.
    let stale = app
        .send(
            Method::PATCH,
            &format!("/scim/v2/Users/{id}"),
            &[
                ("authorization", &format!("Bearer {token}")),
                ("if-match", "W/\"1\""),
            ],
            Some(
                json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
                        "Operations": [{"op": "Replace", "path": "title", "value": "x"}]}),
            ),
        )
        .await;
    assert_eq!(
        stale.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        stale.text
    );

    // Disabled ("False", as Entra ID sends it), renamed, enabled.
    let disabled = scim(
        Method::PATCH,
        format!("/scim/v2/Users/{id}"),
        Some(
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
                    "Operations": [{"op": "Replace", "path": "active", "value": "False"}]}),
        ),
    )
    .await;
    assert_eq!(disabled.json["active"], false, "{}", disabled.text);
    let fcp = app
        .admin(Method::GET, &format!("/api/v1/users/{user_name}"), None)
        .await;
    assert_eq!(fcp.json["account_status"], "suspended");
    let renamed = scim(
        Method::PATCH,
        format!("/scim/v2/Users/{id}"),
        Some(
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [
                {"op": "Replace", "path": "userName", "value": "renamed.user@testuser.com"},
                {"op": "Replace", "path": "active", "value": "True"}
            ]}),
        ),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.text);
    assert_eq!(renamed.json["userName"], "renamed.user@testuser.com");
    assert_eq!(renamed.json["active"], true);
    assert_eq!(
        renamed.json["urn:ietf:params:scim:schemas:extension:fcp:2.0:User"]["sipUsername"],
        user_name,
        "the phones keep their name"
    );

    // Removed: deprovisioned, and gone to SCIM.
    let removed = scim(Method::DELETE, format!("/scim/v2/Users/{id}"), None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.text);
    let gone = scim(Method::GET, format!("/scim/v2/Users/{id}"), None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.json["status"], "404");
    let fcp = app
        .admin(Method::GET, "/api/v1/users/renamed.user@testuser.com", None)
        .await;
    assert_eq!(fcp.json["account_status"], "deprovisioned");

    // A token without scim_provision is refused.
    let reader = app
        .admin(
            Method::POST,
            "/api/v1/api-tokens",
            Some(json!({"name": "reader", "permissions": ["user_read"]})),
        )
        .await;
    let reader = reader.json["secret"].as_str().unwrap().to_string();
    let refused = app
        .send_as(&reader, Method::GET, "/scim/v2/Users", None)
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);

    app.finish().await;
}

#[tokio::test]
async fn okta_provisions_replaces_and_deactivates_a_user() {
    let app = require_app!();
    let token = app.scim_token().await;
    let scim = |method: Method, path: String, body: Option<Value>| {
        let (app, token) = (&app, token.clone());
        async move { app.send_as(&token, method, &path, body).await }
    };
    let lookup = scim(
        Method::GET,
        "/scim/v2/Users?filter=userName%20eq%20%22jane@acme.test%22&startIndex=1&count=100".into(),
        None,
    )
    .await;
    assert_eq!(lookup.json["totalResults"], 0, "{}", lookup.text);

    // Okta's create carries a password, which FCP does not take: the
    // person signs in through the provider, and their SIP password is
    // generated.
    let made = scim(
        Method::POST,
        "/scim/v2/Users".into(),
        Some(json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
            "userName": "jane@acme.test",
            "name": {"givenName": "Jane", "familyName": "Doe"},
            "emails": [{"primary": true, "value": "jane@acme.test", "type": "work"}],
            "displayName": "Jane Doe",
            "locale": "en-US",
            "externalId": "00ujl29u0le5T6Aj10h7",
            "groups": [],
            "password": "1mz050nq",
            "active": true,
            "urn:ietf:params:scim:schemas:extension:fcp:2.0:User": {"extension": "2201", "role": "read_only"}
        })),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let id = scim_user_id(&made);
    assert_eq!(
        app.login("jane@acme.test", "1mz050nq").await.status,
        StatusCode::UNAUTHORIZED,
        "the provisioner's password is not a web password"
    );
    let fcp = app
        .admin(Method::GET, "/api/v1/users/jane@acme.test", None)
        .await;
    assert_eq!(fcp.json["extension"], "2201", "{}", fcp.text);
    assert_eq!(fcp.json["role"], "read_only");

    // Okta replaces the whole user on an update.
    let mut replaced = made.json.clone();
    replaced["name"] = json!({"givenName": "Jane", "familyName": "Smith"});
    replaced["displayName"] = json!("Jane Smith");
    let put = scim(Method::PUT, format!("/scim/v2/Users/{id}"), Some(replaced)).await;
    assert_eq!(put.status, StatusCode::OK, "{}", put.text);
    assert_eq!(put.json["displayName"], "Jane Smith");
    assert_eq!(
        put.json["urn:ietf:params:scim:schemas:extension:fcp:2.0:User"]["extension"],
        "2201"
    );

    // Deactivated and reactivated, as Okta does: a value object.
    for active in [false, true] {
        let res = scim(
            Method::PATCH,
            format!("/scim/v2/Users/{id}"),
            Some(
                json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
                        "Operations": [{"op": "replace", "value": {"active": active}}]}),
            ),
        )
        .await;
        assert_eq!(res.json["active"], active, "{}", res.text);
    }

    // Paging.
    for n in 0..3 {
        scim(
            Method::POST,
            "/scim/v2/Users".into(),
            Some(
                json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
                        "userName": format!("paged{n}@acme.test")}),
            ),
        )
        .await;
    }
    let page = scim(
        Method::GET,
        "/scim/v2/Users?startIndex=2&count=2".into(),
        None,
    )
    .await;
    assert_eq!(page.json["startIndex"], 2, "{}", page.text);
    assert_eq!(page.json["itemsPerPage"], 2);
    assert!(page.json["totalResults"].as_u64().unwrap() >= 4);
    let bad = scim(
        Method::GET,
        "/scim/v2/Users?filter=title%20sw%20%22x%22".into(),
        None,
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.json["scimType"], "invalidFilter");

    app.finish().await;
}

#[tokio::test]
async fn a_tenant_whose_users_come_from_scim_signs_in_only_those() {
    let app = require_app!();
    let idp = StandInIdp::start(IDP_CLIENT, Some(IDP_SECRET))
        .await
        .unwrap();
    app.admin(
        Method::PUT,
        &format!("/api/v1/tenants/{}", app.tenant_a),
        Some(json!({"login_domains": ["acme.test"],
                    "sign_in": {"password_login": "allowed", "require_totp": "none", "scim": true}})),
    )
    .await;
    let provider = app
        .make_provider(&idp, json!({"jit": true, "link_by_email": true}))
        .await;
    let error = |landing: &str| query_param(landing, "sso_error").unwrap_or_default();

    // Made by hand, not by SCIM: refused.
    app.admin(
        Method::POST,
        "/api/v1/users",
        Some(json!({"username": "manual", "email": "manual@acme.test"})),
    )
    .await;
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "m", "email": "manual@acme.test", "email_verified": true}),
        )
        .await;
    assert_eq!(error(&landing), "not_provisioned");

    // A stranger: not made just in time.
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "s", "email": "stranger@acme.test", "email_verified": true,
                   "preferred_username": "stranger@acme.test"}),
        )
        .await;
    assert_eq!(error(&landing), "not_provisioned");

    // Made by SCIM: signs in.
    let token = app.scim_token().await;
    let made = app
        .send_as(
            &token,
            Method::POST,
            "/scim/v2/Users",
            Some(
                json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
                        "userName": "sara", "emails": [{"value": "sara@acme.test"}]}),
            ),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let landing = app
        .sso_landing(
            &idp,
            &provider,
            json!({"sub": "sa", "email": "sara@acme.test", "email_verified": true}),
        )
        .await;
    let signed_in = app.sso_exchange(&landing).await;
    assert_eq!(
        signed_in.status,
        StatusCode::OK,
        "{landing} {}",
        signed_in.text
    );
    assert_eq!(signed_in.json["user"]["username"], "sara");

    app.finish().await;
}

#[tokio::test]
async fn entra_id_and_okta_provision_groups_and_their_members() {
    let app = require_app!();
    let token = app.scim_token().await;
    let scim = |method: Method, path: String, body: Option<Value>| {
        let (app, token) = (&app, token.clone());
        async move { app.send_as(&token, method, &path, body).await }
    };
    let make_user = |name: &'static str| {
        let scim = &scim;
        async move {
            let res = scim(
                Method::POST,
                "/scim/v2/Users".into(),
                Some(
                    json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
                            "userName": name}),
                ),
            )
            .await;
            assert_eq!(res.status, StatusCode::CREATED, "{}", res.text);
            scim_user_id(&res)
        }
    };
    let ana = make_user("ana").await;
    let bo = make_user("bo").await;
    let patch = |ops: Value| json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": ops});
    let member_ids = |res: &Response| -> Vec<String> {
        let mut ids: Vec<String> = res.json["members"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|m| m["value"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };

    // ── Entra ID: an empty group, looked up without members, then filled
    // and emptied one PATCH at a time.
    let made = scim(
        Method::POST,
        "/scim/v2/Groups".into(),
        Some(json!({
            "externalId": "8aa1a0c0-c4c3-4bc0-b4a5-2ef676900159",
            "displayName": "Group1936FD6D",
            "meta": {"resourceType": "Group"},
            "members": [],
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"]
        })),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let entra_group = made.json["id"].as_str().unwrap().to_string();
    assert!(entra_group.starts_with("grp-"), "{entra_group}");
    let found = scim(
        Method::GET,
        "/scim/v2/Groups?excludedAttributes=members&filter=displayName%20eq%20%22Group1936FD6D%22"
            .into(),
        None,
    )
    .await;
    assert_eq!(found.json["totalResults"], 1, "{}", found.text);
    assert!(found.json["Resources"][0].get("members").is_none());
    let added = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{entra_group}"),
        Some(patch(json!([{"op": "Add", "path": "members",
                           "value": [{"value": ana}, {"value": bo}]}]))),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text);
    let mut both = vec![ana.clone(), bo.clone()];
    both.sort();
    assert_eq!(member_ids(&added), both);
    let removed = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{entra_group}"),
        Some(patch(
            json!([{"op": "Remove", "path": "members", "value": [{"value": ana}]}]),
        )),
    )
    .await;
    assert_eq!(member_ids(&removed), vec![bo.clone()], "{}", removed.text);
    let renamed = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{entra_group}"),
        Some(patch(
            json!([{"op": "Replace", "path": "displayName", "value": "Renamed Group"}]),
        )),
    )
    .await;
    assert_eq!(
        renamed.json["displayName"], "Renamed Group",
        "{}",
        renamed.text
    );
    assert_eq!(member_ids(&renamed), vec![bo.clone()]);
    let stranger = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{entra_group}"),
        Some(patch(json!([{"op": "Add", "path": "members",
                           "value": [{"value": "user-00000000-0000-0000-0000-000000000000"}]}]))),
    )
    .await;
    assert_eq!(
        stranger.status,
        StatusCode::BAD_REQUEST,
        "{}",
        stranger.text
    );

    // ── Okta: a group made with its members; one removed by a filtered
    // path, the name replaced by a value object, then the whole group.
    let made = scim(
        Method::POST,
        "/scim/v2/Groups".into(),
        Some(json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
            "displayName": "Test SCIMv2",
            "members": [{"value": ana, "display": "ana"}, {"value": bo, "display": "bo"}]
        })),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let okta_group = made.json["id"].as_str().unwrap().to_string();
    assert_eq!(member_ids(&made), both);
    let dup = scim(
        Method::POST,
        "/scim/v2/Groups".into(),
        Some(
            json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
                    "displayName": "test scimv2"}),
        ),
    )
    .await;
    assert_eq!(dup.json["scimType"], "uniqueness", "{}", dup.text);
    let removed = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{okta_group}"),
        Some(patch(
            json!([{"op": "remove", "path": format!("members[value eq \"{bo}\"]")}]),
        )),
    )
    .await;
    assert_eq!(member_ids(&removed), vec![ana.clone()], "{}", removed.text);
    let renamed = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{okta_group}"),
        Some(patch(json!([{"op": "replace",
                           "value": {"id": okta_group, "displayName": "Okta Renamed"}}]))),
    )
    .await;
    assert_eq!(
        renamed.json["displayName"], "Okta Renamed",
        "{}",
        renamed.text
    );
    let put = scim(
        Method::PUT,
        format!("/scim/v2/Groups/{okta_group}"),
        Some(
            json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
                    "displayName": "Okta Renamed", "members": [{"value": bo}]}),
        ),
    )
    .await;
    assert_eq!(member_ids(&put), vec![bo.clone()], "{}", put.text);

    // A role carried on FCP's extension is the members' too.
    let role = scim(
        Method::PATCH,
        format!("/scim/v2/Groups/{okta_group}"),
        Some(patch(json!([{"op": "add",
                           "path": "urn:ietf:params:scim:schemas:extension:fcp:2.0:Group:role",
                           "value": "user_admin"}]))),
    )
    .await;
    assert_eq!(
        role.json["urn:ietf:params:scim:schemas:extension:fcp:2.0:Group"]["role"], "user_admin",
        "{}",
        role.text
    );
    let groups_of_bo = app
        .admin(Method::GET, "/api/v1/users/bo/groups", None)
        .await;
    assert_eq!(
        groups_of_bo.json.as_array().unwrap().len(),
        2,
        "{}",
        groups_of_bo.text
    );

    // Deleted: gone to SCIM and to FCP; its members are not.
    let gone = scim(
        Method::DELETE,
        format!("/scim/v2/Groups/{okta_group}"),
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    assert_eq!(
        scim(Method::GET, format!("/scim/v2/Groups/{okta_group}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.admin(Method::GET, "/api/v1/users/bo", None)
            .await
            .status,
        StatusCode::OK
    );
    let types = scim(Method::GET, "/scim/v2/ResourceTypes".into(), None).await;
    assert_eq!(
        types.json["Resources"][1]["endpoint"], "/Groups",
        "{}",
        types.text
    );

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Provisioning that phones can use (docs/IDENTITY_AND_DEVICES.md phase 7a)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_phone_fetches_its_files_from_the_tenants_url_as_its_vendor_names_them() {
    use base64::Engine;
    let app = require_app!();
    let owner = app.create_user("carol", Some("Sip-Secret-123")).await;
    let (device_id, token) = app
        .create_phone("Carol's phone", "AA:BB:CC:DD:EE:07", Some(&owner))
        .await;
    let template = app.create_template(app.tenant_a, TEMPLATE).await;
    app.patch_device(&device_id, json!({ "config_template_id": template }))
        .await;

    // The tenant's URL: none until made.
    assert_eq!(
        app.admin(Method::GET, "/api/v1/provisioning/key", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let made = app
        .admin(Method::POST, "/api/v1/provisioning/key", None)
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let key = made.json["key"].as_str().unwrap().to_string();
    assert_eq!(made.json["path"], format!("/provision/{key}/"));
    let shown = app
        .admin(Method::GET, "/api/v1/provisioning/key", None)
        .await;
    assert_eq!(shown.json["key"], key.as_str(), "shown again");

    let fetch = |key: String, file: &'static str, extra: Vec<(&'static str, String)>| {
        let app = &app;
        async move {
            let mut headers: Vec<(&str, String)> = vec![("x-forwarded-proto", "https".into())];
            headers.extend(extra);
            let headers: Vec<(&str, &str)> =
                headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            app.send(
                Method::GET,
                &format!("/provision/{key}/{file}"),
                &headers,
                None,
            )
            .await
        }
    };

    // Each vendor's name for the phone's file, with its content type.
    for (file, content_type) in [
        ("aabbccddee07.cfg", "text/plain"),
        ("cfgaabbccddee07.xml", "application/xml"),
        ("SEPAABBCCDDEE07.cnf.xml", "application/xml"),
        ("spaaabbccddee07.xml", "application/xml"),
        ("snomD785-AABBCCDDEE07.htm", "text/html"),
        ("aabbccddee07", "text/plain"),
    ] {
        let res = fetch(key.clone(), file, vec![]).await;
        assert_eq!(res.status, StatusCode::OK, "{file}: {}", res.text);
        assert!(
            res.header("content-type")
                .is_some_and(|ct| ct.starts_with(content_type)),
            "{file}: {:?}",
            res.header("content-type")
        );
        assert!(
            res.text.contains("account.1.user_name=carol"),
            "{file}: {}",
            res.text
        );
        assert_eq!(
            res.header("cache-control"),
            Some("no-store, no-cache, max-age=0")
        );
    }

    // A Polycom asks for a master file first, which names its own.
    let polycom = vec![(
        "user-agent",
        "FileTransport PolycomVVX-VVX_450-UA/6.4.0.4006".to_string(),
    )];
    let master = fetch(key.clone(), "aabbccddee07.cfg", polycom.clone()).await;
    assert_eq!(master.status, StatusCode::OK, "{}", master.text);
    assert!(
        master
            .text
            .contains("CONFIG_FILES=\"aabbccddee07-fcp.cfg\""),
        "{}",
        master.text
    );
    let own = fetch(key.clone(), "aabbccddee07-fcp.cfg", polycom).await;
    assert!(
        own.text.contains("account.1.user_name=carol"),
        "{}",
        own.text
    );

    // What is not served: a vendor's common file, another MAC, another
    // key, plain HTTP.
    for (key, file) in [
        (key.clone(), "y000000000096.cfg"),
        (key.clone(), "001122334455.cfg"),
        (key.clone(), "../../etc/passwd"),
        ("0".repeat(32), "aabbccddee07.cfg"),
    ] {
        let res = fetch(key, file, vec![]).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{file}");
    }
    let plain = app
        .send(
            Method::GET,
            &format!("/provision/{key}/aabbccddee07.cfg"),
            &[],
            None,
        )
        .await;
    assert_eq!(plain.status, StatusCode::NOT_FOUND, "HTTPS only");

    // The tenant wants each phone's own credentials: the provisioning
    // token, as the Basic password; and its networks only.
    let set = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({"provisioning": {"require_device_credentials": true,
                                         "networks": ["10.9.0.0/16"]}})),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let basic = |password: &str| {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("aabbccddee07:{password}"))
        )
    };
    let inside = ("x-forwarded-for", "10.9.4.20".to_string());
    let asked = fetch(key.clone(), "aabbccddee07.cfg", vec![inside.clone()]).await;
    assert_eq!(asked.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        asked.header("www-authenticate"),
        Some("Basic realm=\"FCP provisioning\"")
    );
    let wrong = fetch(
        key.clone(),
        "aabbccddee07.cfg",
        vec![inside.clone(), ("authorization", basic("nope"))],
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let right = fetch(
        key.clone(),
        "aabbccddee07.cfg",
        vec![inside.clone(), ("authorization", basic(&token))],
    )
    .await;
    assert_eq!(right.status, StatusCode::OK, "{}", right.text);
    let outside = fetch(
        key.clone(),
        "aabbccddee07.cfg",
        vec![
            ("x-forwarded-for", "192.0.2.1".to_string()),
            ("authorization", basic(&token)),
        ],
    )
    .await;
    assert_eq!(outside.status, StatusCode::NOT_FOUND, "another network");

    // A new key: the old URL is dead.
    let rotated = app
        .admin(Method::POST, "/api/v1/provisioning/key", None)
        .await;
    let new_key = rotated.json["key"].as_str().unwrap().to_string();
    assert_ne!(new_key, key);
    let old = fetch(
        key.clone(),
        "aabbccddee07.cfg",
        vec![inside.clone(), ("authorization", basic(&token))],
    )
    .await;
    assert_eq!(old.status, StatusCode::NOT_FOUND);
    let new = fetch(
        new_key,
        "aabbccddee07.cfg",
        vec![inside, ("authorization", basic(&token))],
    )
    .await;
    assert_eq!(new.status, StatusCode::OK);

    // Every fetch is on the device.
    let fetches = app
        .admin(
            Method::GET,
            &format!("/api/v1/devices/{device_id}/fetches"),
            None,
        )
        .await;
    assert_eq!(fetches.status, StatusCode::OK, "{}", fetches.text);
    let outcomes: Vec<&str> = fetches
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(outcomes[0], "served", "{outcomes:?}");
    for want in ["served", "unauthorized", "network_refused"] {
        assert!(outcomes.contains(&want), "{want} in {outcomes:?}");
    }
    assert!(fetches
        .json
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["user_agent"]
            .as_str()
            .is_some_and(|ua| ua.contains("Polycom"))));

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Provisioning: templates by model, pending phones, import (phase 7b)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pending_phone_is_claimed_and_gets_the_template_for_its_model() {
    let app = require_app!();
    app.create_user("dave", Some("Sip-Secret-456")).await;
    let key = app
        .admin(Method::POST, "/api/v1/provisioning/key", None)
        .await
        .json["key"]
        .as_str()
        .unwrap()
        .to_string();
    let template = |name: &'static str,
                    vendor: &'static str,
                    pattern: Option<&'static str>,
                    body: &'static str| {
        let app = &app;
        async move {
            app.admin(
                Method::POST,
                "/api/v1/config-templates",
                Some(
                    json!({"name": name, "vendor": vendor, "model_pattern": pattern,
                            "template_body": body}),
                ),
            )
            .await
        }
    };
    // Two Yealink templates by model, and the T5's uses Jinja.
    for (name, pattern, body) in [
        ("t4", "T4*", "t4 {{ sip_username }}\n"),
        (
            "t5",
            "T5*",
            "{% for a in accounts %}account.{{ a.index }}.user_name = {{ a.user_name }}\n{% endfor %}\
             {% for k in line_keys %}linekey.{{ k.index }} = {{ k.kind }}:{{ k.value }}:{{ k.label }}\n{% endfor %}\
             transport={{ sip_transport }}:{{ sip_port }} tz={{ timezone }}\n\
             url={{ provisioning_url }}\nlegacy=${sip_username}\n",
        ),
    ] {
        let made = template(name, "yealink", Some(pattern), body).await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    }
    // A template that does not parse is refused, naming the line.
    let broken = template("broken", "yealink", None, "a\n{% for x in accounts %}\n").await;
    assert_eq!(broken.status, StatusCode::BAD_REQUEST);
    assert!(broken.text.contains("line"), "{}", broken.text);

    let fetch = |file: String, ua: &'static str| {
        let app = &app;
        let key = key.clone();
        async move {
            app.send(
                Method::GET,
                &format!("/provision/{key}/{file}"),
                &[("x-forwarded-proto", "https"), ("user-agent", ua)],
                None,
            )
            .await
        }
    };
    const T54W: &str = "Yealink SIP-T54W 96.86.0.70 80:5e:c0:d1:e2:f3";

    // A phone out of its box asks: nobody has its MAC, so it waits.
    for _ in 0..2 {
        let res = fetch("805ec0d1e2f3.cfg".into(), T54W).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
    }
    let pending = app
        .admin(Method::GET, "/api/v1/provisioning/pending", None)
        .await;
    assert_eq!(pending.status, StatusCode::OK, "{}", pending.text);
    let rows = pending.json.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["mac_address"], "805ec0d1e2f3");
    assert_eq!(rows[0]["vendor"], "yealink");
    assert_eq!(rows[0]["model"], "T54W");
    assert_eq!(rows[0]["firmware"], "96.86.0.70");
    assert_eq!(rows[0]["fetches"], 2);

    // Claimed in one step, for dave.
    let claimed = app
        .admin(
            Method::POST,
            "/api/v1/provisioning/pending/80:5E:C0:D1:E2:F3/claim",
            Some(json!({"owner_username": "dave"})),
        )
        .await;
    assert_eq!(claimed.status, StatusCode::CREATED, "{}", claimed.text);
    let device_id = claimed.json["id"].as_str().unwrap().to_string();
    assert_eq!(claimed.json["vendor"], "yealink");
    assert_eq!(claimed.json["model"], "T54W");
    assert_eq!(claimed.json["firmware_version"], "96.86.0.70");
    assert_eq!(claimed.json["name"], "Yealink T54W 805ec0d1e2f3");
    assert!(claimed.json["provisioning_token"].is_string(), "shown once");
    let again = app
        .admin(
            Method::POST,
            "/api/v1/provisioning/pending/805ec0d1e2f3/claim",
            None,
        )
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND, "no longer pending");

    // Its keys.
    let bad = app
        .patch_device(
            &device_id,
            json!({"line_keys": [{"kind": "blf", "value": " "}]}),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let keyed = app
        .patch_device(
            &device_id,
            json!({"line_keys": [{"kind": "blf", "value": "202", "label": "Bob"},
                                 {"kind": "park", "value": "701"}]}),
        )
        .await;
    assert_eq!(keyed.status, StatusCode::OK, "{}", keyed.text);
    assert_eq!(keyed.json["line_keys"][1]["kind"], "park");

    // Now it gets the T5 template, rendered.
    let served = fetch("805ec0d1e2f3.cfg".into(), T54W).await;
    assert_eq!(served.status, StatusCode::OK, "{}", served.text);
    for want in [
        "account.1.user_name = dave\n",
        "linekey.1 = blf:202:Bob\n",
        "linekey.2 = park:701:701\n",
        "transport=udp:5060 tz=UTC\n",
        &format!("url={SSO_PUBLIC_URL}/provision/{key}/\n"),
        "legacy=dave\n",
    ] {
        assert!(served.text.contains(want), "{want:?} in\n{}", served.text);
    }
    // The same preview, for an administrator; and an unsaved body.
    let preview = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{device_id}/render"),
            None,
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json["config_template_name"], "t5");
    assert!(preview.json["content"]
        .as_str()
        .unwrap()
        .contains("linekey.1 = blf:202:Bob"));
    let tried = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{device_id}/render"),
            Some(json!({"template_body": "{{ sip_server }} {{ model }} {{ vendor }}"})),
        )
        .await;
    assert_eq!(tried.status, StatusCode::OK, "{}", tried.text);
    assert_eq!(tried.json["content"], format!("{REALM} T54W yealink"));
    let failing = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{device_id}/render"),
            Some(json!({"template_body": "{{ model|no_such_filter }}"})),
        )
        .await;
    assert_eq!(failing.status, StatusCode::BAD_REQUEST, "{}", failing.text);

    // A T4 phone the admin made by hand gets the T4 one.
    let t46 = app
        .admin(
            Method::POST,
            "/api/v1/devices",
            Some(json!({"name": "T46", "device_type": "desk_phone", "mac_address": "00:15:65:00:00:46",
                        "model": "T46U", "owner_user_id": claimed.json["owner_user_id"]})),
        )
        .await;
    assert_eq!(t46.status, StatusCode::CREATED, "{}", t46.text);
    assert_eq!(t46.json["vendor"], "yealink", "from the MAC's prefix");
    let res = fetch("001565000046.cfg".into(), "Yealink SIP-T46U 108.86.0.45").await;
    assert_eq!(res.text, "t4 dave\n");

    // Dismissed, a pending phone is gone until it asks again.
    fetch("0c383e000001.cfg".into(), "Fanvil X4U 2.12.1").await;
    let gone = app
        .admin(
            Method::DELETE,
            "/api/v1/provisioning/pending/0c383e000001",
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    let twice = app
        .admin(
            Method::DELETE,
            "/api/v1/provisioning/pending/0c383e000001",
            None,
        )
        .await;
    assert_eq!(twice.status, StatusCode::NOT_FOUND);

    app.finish().await;
}

#[tokio::test]
async fn phones_are_imported_all_or_none_and_xml_values_are_escaped() {
    let app = require_app!();
    app.create_user("erin", Some("Sip-Secret-789")).await;
    let made = app
        .admin(
            Method::POST,
            "/api/v1/config-templates",
            Some(json!({"name": "vvx", "vendor": "polycom", "model_pattern": "VVX*",
                        "template_body": "<x name=\"{{ device_name }}\" user=\"{{ sip_username }}\" raw=\"{{ '<b/>'|safe }}\"/>\n"})),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);

    // One bad row and nothing is made; every fault named.
    let refused = app
        .admin(
            Method::POST,
            "/api/v1/devices/import",
            Some(json!({"devices": [
                {"mac_address": "0004f2aaaa01", "name": "Lab & Co", "model": "VVX 450"},
                {"mac_address": "nope"},
                {"mac_address": "00:04:F2:AA:AA:01"},
                {"mac_address": "0004f2aaaa03", "owner_username": "nobody"},
            ]})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    for want in ["row 2", "row 3", "twice", "row 4", "nobody"] {
        assert!(refused.text.contains(want), "{want} in {}", refused.text);
    }
    let none = app.admin(Method::GET, "/api/v1/devices", None).await;
    assert_eq!(none.json.as_array().unwrap().len(), 0, "{}", none.text);

    let imported = app
        .admin(
            Method::POST,
            "/api/v1/devices/import",
            Some(json!({"devices": [
                {"mac_address": "0004f2aaaa01", "name": "Lab & Co", "model": "VVX 450"},
                {"mac_address": "00-04-f2-aa-aa-02", "owner_username": "erin", "model": "VVX 250"},
            ]})),
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.text);
    let created = imported.json["created"].as_array().unwrap();
    assert_eq!(created.len(), 2);
    assert!(created
        .iter()
        .all(|d| d["vendor"] == "polycom" && d["provisioning_token"].is_string()));
    assert_eq!(created[1]["mac_address"], "0004f2aaaa02");
    let erins = created[1]["id"].as_str().unwrap();
    let labs = created[0]["id"].as_str().unwrap();
    let dup = app
        .admin(
            Method::POST,
            "/api/v1/devices/import",
            Some(json!({"devices": [{"mac_address": "0004f2aaaa02"}]})),
        )
        .await;
    assert_eq!(dup.status, StatusCode::BAD_REQUEST, "{}", dup.text);

    // XML vendors' values are escaped; `|safe` is not.
    let lab = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{labs}/render"),
            None,
        )
        .await;
    assert_eq!(lab.status, StatusCode::OK, "{}", lab.text);
    assert_eq!(
        lab.json["content"],
        "<x name=\"Lab &amp; Co\" user=\"\" raw=\"<b/>\"/>\n"
    );
    let erin = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{erins}/render"),
            None,
        )
        .await;
    assert!(erin.json["content"]
        .as_str()
        .unwrap()
        .contains("user=\"erin\""));

    // A body tried against the sample values.
    let sample = app
        .admin(
            Method::POST,
            "/api/v1/config-templates/preview",
            Some(json!({"template_body": "{{ owner_display_name }}|{{ accounts|length }}", "vendor": "polycom"})),
        )
        .await;
    assert_eq!(sample.status, StatusCode::OK, "{}", sample.text);
    assert_eq!(sample.json["content"], "Alice O&apos;Neil &amp; Co|2");
    assert!(sample.json["context"]["line_keys"].is_array());

    // Every vendor has a starter, and each renders for a real device.
    let library = app
        .admin(Method::GET, "/api/v1/config-templates/library", None)
        .await;
    let vendors: Vec<&str> = library
        .json
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["vendor"].as_str())
        .collect();
    for v in [
        "yealink",
        "polycom",
        "grandstream",
        "cisco",
        "fanvil",
        "snom",
    ] {
        assert!(vendors.contains(&v), "{v} in {vendors:?}");
    }
    let starter = app
        .admin(
            Method::POST,
            "/api/v1/config-templates/library/polycom",
            None,
        )
        .await;
    assert_eq!(starter.status, StatusCode::CREATED, "{}", starter.text);
    let rendered = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{erins}/render"),
            Some(json!({"config_template_id": starter.json["id"]})),
        )
        .await;
    assert_eq!(rendered.status, StatusCode::OK, "{}", rendered.text);
    assert!(rendered.json["content"]
        .as_str()
        .unwrap()
        .contains("reg.1.address=\"erin@"));

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Zero-touch: vendors' redirection services (docs/IDENTITY_AND_DEVICES.md phase 8)
// ---------------------------------------------------------------------------

/// Poll a device's redirection until `done` says it has landed.
async fn redirection_of(app: &App, device_id: &str, done: impl Fn(&Value) -> bool) -> Value {
    let mut last = Value::Null;
    for _ in 0..50 {
        let res = app
            .admin(
                Method::GET,
                &format!("/api/v1/devices/{device_id}/redirection"),
                None,
            )
            .await;
        if res.status == StatusCode::OK && done(&res.json) {
            return res.json;
        }
        last = res.json;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the device's redirection never settled: {last}");
}

#[tokio::test]
async fn a_phone_is_registered_with_its_vendors_service_and_removed_with_its_device() {
    use base64::Engine;
    use fcp_testkit::redirection::{StandInRps, StandInZtp};
    let app = require_app!();
    let rps = StandInRps::start("rps-key-id", "rps-key-secret")
        .await
        .unwrap();
    let ztp = StandInZtp::start("ztp-user", "ztp-pass", &["acme-ref"])
        .await
        .unwrap();
    let app = app.with_redirection(fcp_core::ApiRedirectionConfig {
        yealink_rps_url: rps.url().to_string(),
        poly_ztp_url: ztp.url().to_string(),
    });
    let key = app.provisioning_key().await;
    // The tenant wants each phone's own credentials: RPS hands them over.
    let set = app
        .admin(
            Method::PUT,
            &format!("/api/v1/tenants/{}", app.tenant_a),
            Some(json!({"provisioning": {"require_device_credentials": true}})),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text);
    let owner = app.create_user("fay", Some("Sip-Secret-321")).await;
    let template = app.create_template(app.tenant_a, TEMPLATE).await;

    // The accounts: a wrong secret is told, and a Poly account needs its
    // Reference ID and profile.
    let wrong = app
        .admin(
            Method::PUT,
            "/api/v1/provisioning/redirection/yealink_rps",
            Some(json!({"key_id": "rps-key-id", "secret": "not-it"})),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::OK, "{}", wrong.text);
    assert!(
        wrong.json.get("secret").is_none(),
        "the secret is never shown"
    );
    let checked = app
        .admin(
            Method::POST,
            "/api/v1/provisioning/redirection/yealink_rps/check",
            None,
        )
        .await;
    assert_eq!(checked.json["ok"], false, "{}", checked.text);
    assert!(
        checked.json["detail"].as_str().unwrap().contains("401"),
        "{}",
        checked.text
    );
    let right = app
        .admin(
            Method::PUT,
            "/api/v1/provisioning/redirection/yealink_rps",
            Some(json!({"key_id": "rps-key-id", "secret": "rps-key-secret"})),
        )
        .await;
    assert_eq!(right.status, StatusCode::OK, "{}", right.text);
    let checked = app
        .admin(
            Method::POST,
            "/api/v1/provisioning/redirection/yealink_rps/check",
            None,
        )
        .await;
    assert_eq!(checked.json["ok"], true, "{}", checked.text);
    let incomplete = app
        .admin(
            Method::PUT,
            "/api/v1/provisioning/redirection/poly_ztp",
            Some(json!({"key_id": "ztp-user", "secret": "ztp-pass"})),
        )
        .await;
    assert_eq!(
        incomplete.status,
        StatusCode::BAD_REQUEST,
        "{}",
        incomplete.text
    );
    let poly = app
        .admin(
            Method::PUT,
            "/api/v1/provisioning/redirection/poly_ztp",
            Some(json!({"key_id": "ztp-user", "secret": "ztp-pass",
                        "account_id": "acme-ref", "profile": "acme-fcp"})),
        )
        .await;
    assert_eq!(poly.status, StatusCode::OK, "{}", poly.text);
    let listed = app
        .admin(Method::GET, "/api/v1/provisioning/redirection", None)
        .await;
    assert_eq!(listed.json.as_array().unwrap().len(), 2, "{}", listed.text);

    // A Yealink made by MAC is registered: its MAC, the tenant's URL, and
    // credentials the provisioning URL takes.
    let (yealink, _token) = app
        .create_phone("Fay's desk", "80:5E:C0:11:22:33", Some(&owner))
        .await;
    app.patch_device(&yealink, json!({ "config_template_id": template }))
        .await;
    let record = redirection_of(&app, &yealink, |r| r["state"] == "registered").await;
    assert_eq!(record["service"], "yealink_rps");
    assert_eq!(record["with_credentials"], true);
    let at_rps = rps.device("805ec0112233").expect("RPS has the phone");
    assert_eq!(at_rps.url, format!("{SSO_PUBLIC_URL}/provision/{key}/"));
    let password = at_rps.password.clone().expect("credentials for the phone");
    assert!(password.len() <= 32, "{password}");
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{password}", at_rps.auth_name.unwrap()))
    );
    let fetched = app
        .send(
            Method::GET,
            &format!("/provision/{key}/805ec0112233.cfg"),
            &[
                ("x-forwarded-proto", "https"),
                ("authorization", basic.as_str()),
            ],
            None,
        )
        .await;
    assert_eq!(
        fetched.status,
        StatusCode::OK,
        "the credentials RPS hands the phone work: {}",
        fetched.text
    );

    // A Polycom is associated with the tenant's profile.
    let (polycom, _) = app
        .create_phone("Fay's VVX", "00:04:F2:11:22:33", Some(&owner))
        .await;
    let record = redirection_of(&app, &polycom, |r| r["state"] == "registered").await;
    assert_eq!(record["service"], "poly_ztp");
    assert_eq!(record["with_credentials"], false);
    assert_eq!(
        ztp.device("0004f2112233"),
        Some(fcp_testkit::redirection::ZtpDevice {
            account_id: "acme-ref".to_string(),
            profile: "acme-fcp".to_string(),
        })
    );

    // A phone of another vendor has no service.
    let (other, _) = app
        .create_phone("Lab phone", "AA:BB:CC:00:00:01", Some(&owner))
        .await;
    let none = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{other}/redirection"),
            None,
        )
        .await;
    assert_eq!(none.status, StatusCode::BAD_REQUEST, "{}", none.text);

    // A new MAC moves it; deleting the device takes it off.
    let moved = app
        .patch_device(&yealink, json!({"mac_address": "80:5E:C0:44:55:66"}))
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text);
    redirection_of(&app, &yealink, |r| r["mac_address"] == "805ec0445566").await;
    assert!(rps.device("805ec0112233").is_none());
    assert!(rps.deleted().contains(&"805ec0112233".to_string()));
    assert!(rps.device("805ec0445566").is_some());
    for id in [&yealink, &polycom] {
        let gone = app
            .admin(Method::DELETE, &format!("/api/v1/devices/{id}"), None)
            .await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    }
    for _ in 0..50 {
        if rps.device("805ec0445566").is_none() && ztp.device("0004f2112233").is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(rps.device("805ec0445566").is_none(), "off RPS");
    assert!(ztp.device("0004f2112233").is_none(), "off ZTP");

    // An account made after its phones registers them on a sync; a
    // refusal is kept on the device.
    let (late, _) = app
        .create_phone("Late Yealink", "00:15:65:00:00:99", Some(&owner))
        .await;
    redirection_of(&app, &late, |r| r["state"] == "registered").await;
    let off = app
        .admin(
            Method::PUT,
            "/api/v1/provisioning/redirection/yealink_rps",
            Some(json!({"key_id": "rps-key-id", "secret": "not-it", "enabled": true})),
        )
        .await;
    assert_eq!(off.status, StatusCode::OK);
    let again = app
        .admin(
            Method::POST,
            &format!("/api/v1/devices/{late}/redirection"),
            None,
        )
        .await;
    assert_eq!(again.json["state"], "failed", "{}", again.text);
    assert!(
        again.json["detail"].as_str().unwrap().contains("401"),
        "{}",
        again.text
    );
    let fixed = app
        .admin(
            Method::PUT,
            "/api/v1/provisioning/redirection/yealink_rps",
            Some(json!({"key_id": "rps-key-id", "secret": "rps-key-secret"})),
        )
        .await;
    assert_eq!(fixed.status, StatusCode::OK);
    let synced = app
        .admin(
            Method::POST,
            "/api/v1/provisioning/redirection/yealink_rps/sync",
            None,
        )
        .await;
    assert_eq!(synced.status, StatusCode::OK, "{}", synced.text);
    assert_eq!(
        (
            synced.json["devices"].as_u64(),
            synced.json["registered"].as_u64()
        ),
        (Some(1), Some(1)),
        "{}",
        synced.text
    );

    app.finish().await;
}

// ---------------------------------------------------------------------------
// Firmware (docs/IDENTITY_AND_DEVICES.md phase 9)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn firmware_is_uploaded_once_targeted_per_tenant_piloted_and_served() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let app = require_app!().with_firmware(dir.path());
    let key = app.provisioning_key().await;
    let owner = app.create_user("gus", Some("Sip-Secret-654")).await;
    let starter = app
        .admin(
            Method::POST,
            "/api/v1/config-templates/library/yealink",
            None,
        )
        .await;
    let template = starter.json["id"].as_str().unwrap().to_string();
    let (pilot, _) = app
        .create_phone("Gus's desk", "80:5E:C0:00:0F:01", Some(&owner))
        .await;
    let (other, _) = app
        .create_phone("Lobby", "80:5E:C0:00:0F:02", Some(&owner))
        .await;
    for id in [&pilot, &other] {
        app.patch_device(id, json!({ "config_template_id": template }))
            .await;
    }
    let config_of = |mac: &'static str| {
        let app = &app;
        let key = key.clone();
        async move {
            app.send(
                Method::GET,
                &format!("/provision/{key}/{mac}.cfg"),
                &[("x-forwarded-proto", "https")],
                None,
            )
            .await
            .text
        }
    };

    // The platform administrator uploads an image; nobody else may.
    let bytes: Vec<u8> = b"FCP test firmware image ".repeat(400);
    let upload =
        |token: &'static str, file: &'static str, version: &'static str, bytes: Vec<u8>| {
            let app = &app;
            async move {
                app.send_multipart(
                    token,
                    "/api/v1/firmware",
                    &[
                        ("vendor", None, b"yealink".as_slice()),
                        ("version", None, version.as_bytes()),
                        ("models", None, b"T5*, T4*".as_slice()),
                        (
                            "file",
                            Some((file, "application/octet-stream")),
                            bytes.as_slice(),
                        ),
                    ],
                )
                .await
            }
        };
    let made = upload(
        ADMIN_TOKEN,
        "T54W-96.86.0.80.rom",
        "96.86.0.80",
        bytes.clone(),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    let image = made.json["id"].as_str().unwrap().to_string();
    assert_eq!(made.json["size"], bytes.len());
    assert_eq!(made.json["sha256"], format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(made.json["models"], json!(["T5*", "T4*"]));
    let bad_name = upload(ADMIN_TOKEN, "..evil", "1.0", bytes.clone()).await;
    assert_eq!(
        bad_name.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_name.text
    );
    let too_big = upload(ADMIN_TOKEN, "big.rom", "1.0", vec![0u8; 1024 * 1024 + 1]).await;
    assert_eq!(too_big.status, StatusCode::BAD_REQUEST, "{}", too_big.text);
    app.create_user_with_role("tina", "tenant_admin").await;
    let tina = app.login("tina", PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = app
        .send_multipart(
            &tina,
            "/api/v1/firmware",
            &[
                ("vendor", None, b"yealink".as_slice()),
                ("version", None, b"9".as_slice()),
                (
                    "file",
                    Some(("x.rom", "application/octet-stream")),
                    b"x".as_slice(),
                ),
            ],
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let other_image = upload(
        ADMIN_TOKEN,
        "T54W-96.86.0.90.rom",
        "96.86.0.90",
        b"newer".to_vec(),
    )
    .await;
    let other_image = other_image.json["id"].as_str().unwrap().to_string();
    let listed = app.admin(Method::GET, "/api/v1/firmware", None).await;
    assert_eq!(listed.json.as_array().unwrap().len(), 2, "{}", listed.text);

    // No target: the phones are told of no firmware.
    assert!(!config_of("805ec0000f01")
        .await
        .contains("static.firmware.url"));

    // Piloted on one phone.
    let target = app
        .admin(
            Method::POST,
            "/api/v1/firmware/targets",
            Some(
                json!({"vendor": "yealink", "model_pattern": "T5*", "image_id": image,
                        "stage": "pilot", "pilot_devices": [pilot]}),
            ),
        )
        .await;
    assert_eq!(target.status, StatusCode::CREATED, "{}", target.text);
    let target_id = target.json["id"].as_str().unwrap().to_string();
    assert_eq!(target.json["version"], "96.86.0.80");
    assert_eq!(
        (
            target.json["devices"].as_u64(),
            target.json["unknown"].as_u64()
        ),
        (Some(2), Some(2)),
        "{}",
        target.text
    );
    let wrong_vendor = app
        .admin(
            Method::POST,
            "/api/v1/firmware/targets",
            Some(json!({"vendor": "polycom", "image_id": image})),
        )
        .await;
    assert_eq!(
        wrong_vendor.status,
        StatusCode::BAD_REQUEST,
        "{}",
        wrong_vendor.text
    );
    let url = format!(
        "{SSO_PUBLIC_URL}/provision/{key}/firmware/{}/T54W-96.86.0.80.rom",
        image.trim_start_matches("fw-").replace('-', "")
    );
    let pilot_config = config_of("805ec0000f01").await;
    assert!(
        pilot_config.contains(&format!("static.firmware.url = {url}")),
        "{pilot_config}"
    );
    assert!(
        !config_of("805ec0000f02")
            .await
            .contains("static.firmware.url"),
        "not in the pilot"
    );

    // The image is served under the tenant's URL, and only it.
    let path = url.trim_start_matches(SSO_PUBLIC_URL).to_string();
    let fetched = app
        .send(Method::GET, &path, &[("x-forwarded-proto", "https")], None)
        .await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.bytes, bytes, "the image as uploaded");
    for wrong in [
        path.replace("T54W-96.86.0.80.rom", "other.rom"),
        format!(
            "/provision/{key}/firmware/{}/T54W-96.86.0.90.rom",
            other_image.trim_start_matches("fw-").replace('-', "")
        ),
        format!(
            "/provision/{}/firmware/{}/T54W-96.86.0.80.rom",
            "0".repeat(32),
            image.trim_start_matches("fw-").replace('-', "")
        ),
    ] {
        let res = app
            .send(Method::GET, &wrong, &[("x-forwarded-proto", "https")], None)
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{wrong}");
    }

    // Rolled out to every phone.
    let all = app
        .admin(
            Method::PUT,
            &format!("/api/v1/firmware/targets/{target_id}"),
            Some(
                json!({"vendor": "yealink", "model_pattern": "T5*", "image_id": image,
                        "stage": "all"}),
            ),
        )
        .await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    assert!(config_of("805ec0000f02")
        .await
        .contains("static.firmware.url"));

    // Who is behind: what each phone runs against the target.
    app.patch_device(&pilot, json!({"firmware_version": "96.86.0.80"}))
        .await;
    app.patch_device(&other, json!({"firmware_version": "96.85.0.5"}))
        .await;
    let behind = app
        .admin(Method::GET, &format!("/api/v1/devices/{other}"), None)
        .await;
    assert_eq!(behind.json["firmware_status"], "behind", "{}", behind.text);
    assert_eq!(behind.json["firmware_target_version"], "96.86.0.80");
    let targets = app
        .admin(Method::GET, "/api/v1/firmware/targets", None)
        .await;
    let row = &targets.json.as_array().unwrap()[0];
    assert_eq!(
        (
            row["current"].as_u64(),
            row["behind"].as_u64(),
            row["unknown"].as_u64()
        ),
        (Some(1), Some(1), Some(0)),
        "{}",
        targets.text
    );
    let devices = app.admin(Method::GET, "/api/v1/devices", None).await;
    assert!(devices
        .json
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["firmware_status"] == "current"));

    // An image a tenant targets stays; untargeted, it goes with its file.
    let kept = app
        .admin(Method::DELETE, &format!("/api/v1/firmware/{image}"), None)
        .await;
    assert_eq!(kept.status, StatusCode::CONFLICT, "{}", kept.text);
    let untarget = app
        .admin(
            Method::DELETE,
            &format!("/api/v1/firmware/targets/{target_id}"),
            None,
        )
        .await;
    assert_eq!(untarget.status, StatusCode::NO_CONTENT);
    assert!(!config_of("805ec0000f01")
        .await
        .contains("static.firmware.url"));
    let gone = app
        .admin(Method::DELETE, &format!("/api/v1/firmware/{image}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT, "{}", gone.text);
    assert!(
        !dir.path()
            .join(image.trim_start_matches("fw-").replace('-', ""))
            .exists(),
        "the file goes too"
    );

    app.finish().await;
}
