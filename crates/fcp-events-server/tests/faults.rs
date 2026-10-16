//! Deep testing D5b (`docs/DEEP_TESTING.md`): the events server taken
//! away while events are flowing, and brought back. Its webhooks and its
//! emergency notifier are the two things it does that nothing else will
//! do in its place, so what each test asks is whether every event that
//! reached the stream reached its receiver.
//!
//! Events go onto the Redis stream as a node puts them there
//! (`RedisEventBus`), and a receiver in the test stands in for a
//! tenant's webhook endpoint, either answering or holding every request
//! open. Needs `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`; skipped
//! otherwise.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use fcp_core::e911::notify::{ContactKind, NotifyContact};
use fcp_core::e911::EmergencyConfig;
use fcp_core::storage::{WebhookEndpoint, WebhookStore};
use fcp_core::{CallId, TenantId, WebhookId};
use fcp_events::{CallDirection, CallEvent, EventPublisher, PlatformEvent};
use fcp_storage_redis::event_bus::RedisEventBus;
use fcp_storage_redis::{RedisConfig, RedisStorage};
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::smoke::{free_port, loopback};
use fcp_testkit::{Seed, Service, TestDatabase};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-events-server");
const TOKEN: &str = "faults-token";
/// How long an entry a dead consumer took waits before another takes it.
const CLAIM_IDLE_SECS: u64 = 3;
/// How long one delivery waits for its receiver.
const REQUEST_TIMEOUT_SECS: u64 = 2;
/// Long enough for a restarted server to claim, deliver and retry.
const A_WHILE: Duration = Duration::from_secs(45);

/// One request the receiver answered.
#[derive(Debug, Clone)]
struct Delivery {
    event_type: String,
    body: serde_json::Value,
}

/// A tenant's webhook endpoint: answers 200, or holds every request open
/// (as a slow receiver does) while `holding`.
#[derive(Clone, Default)]
struct Receiver {
    holding: Arc<AtomicBool>,
    /// Requests that arrived while holding: what the server had taken.
    held: Arc<Mutex<usize>>,
    answered: Arc<Mutex<Vec<Delivery>>>,
}

impl Receiver {
    async fn start() -> (Self, String) {
        let receiver = Self::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/hook", post(receive))
            .with_state(receiver.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (receiver, url)
    }

    fn held(&self) -> usize {
        *self.held.lock().unwrap()
    }

    fn answered(&self) -> Vec<Delivery> {
        self.answered.lock().unwrap().clone()
    }

    /// The call ids each answered `call.initiated` delivery was about,
    /// with the event ids it was sent under.
    fn initiated(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut by_call: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for d in self.answered() {
            if d.event_type != "call.initiated" {
                continue;
            }
            let call = bare(d.body["data"]["call_id"].as_str().unwrap_or(""));
            let id = d.body["event_id"].as_str().unwrap_or("").to_string();
            by_call.entry(call).or_default().insert(id);
        }
        by_call
    }

    /// The call ids of the emergency notices answered.
    fn notified(&self) -> BTreeSet<String> {
        self.answered()
            .iter()
            .filter(|d| d.event_type == "emergency.notification")
            .map(|d| bare(d.body["data"]["notice"]["call_id"].as_str().unwrap_or("")))
            .collect()
    }
}

/// A call id without its `call-` prefix, however it was written.
fn bare(call_id: &str) -> String {
    call_id.strip_prefix("call-").unwrap_or(call_id).to_string()
}

async fn receive(State(receiver): State<Receiver>, headers: HeaderMap, body: String) -> StatusCode {
    if receiver.holding.load(Ordering::Acquire) {
        *receiver.held.lock().unwrap() += 1;
        // Longer than any test: the server's side of the connection goes
        // when it is killed.
        tokio::time::sleep(Duration::from_secs(600)).await;
        return StatusCode::OK;
    }
    let event_type = headers
        .get("X-FCP-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    receiver
        .answered
        .lock()
        .unwrap()
        .push(Delivery { event_type, body });
    StatusCode::OK
}

/// A database with a tenant whose webhook endpoint is the receiver and
/// whose emergency contact is that endpoint; a Redis database; and the
/// events server's configuration over both.
struct Stage {
    db: Option<TestDatabase>,
    seed: Seed,
    redis_url: String,
    receiver: Receiver,
    config: String,
    http: u16,
    bus: RedisEventBus,
    tenant: TenantId,
}

impl Stage {
    async fn start() -> Option<Self> {
        let redis_url = private_redis_url()?;
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), "faults.local").await;
        let tenant = seed.tenant_id;
        let (receiver, hook_url) = Receiver::start().await;

        let endpoint = WebhookEndpoint {
            id: WebhookId::new(),
            tenant_id: tenant,
            url: hook_url,
            secret: "whsec_faults".to_string(),
            events: vec!["call.initiated".to_string()],
            enabled: true,
            description: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            previous_secret: None,
            previous_secret_expires_at: None,
        };
        seed.storage()
            .create_endpoint(endpoint.clone())
            .await
            .expect("webhook endpoint");
        let mut emergency = EmergencyConfig::defaults(tenant);
        emergency.contacts = vec![NotifyContact::new(
            ContactKind::Webhook,
            endpoint.id.as_uuid().to_string(),
        )];
        let emergency_store = fcp_storage_postgres::emergency::PostgresEmergencyStore::new(
            seed.storage().pool().clone(),
        );
        fcp_core::e911::EmergencyStore::put_emergency_config(&emergency_store, &emergency)
            .await
            .expect("emergency config");

        let http = free_port();
        let config = format!(
            r#"[service]
node_id = "faults-events"

[auth]
token = "{TOKEN}"

[websocket]
bind_address = "127.0.0.1:{http}"

[event_bus]
bus_type = "redis"
redis_url = "{redis_url}"
claim_idle_secs = {CLAIM_IDLE_SECS}

[webhook]
enabled = true
database_url = "{db}"
allow_private_targets = true
request_timeout_secs = {REQUEST_TIMEOUT_SECS}
base_retry_delay_secs = 1
retry_interval_secs = 1

[notifications]
retry_interval_secs = 1
base_retry_delay_secs = 1

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
        );
        let redis = RedisStorage::new(RedisConfig {
            url: redis_url.clone(),
            key_prefix: "fcp:".to_string(),
        })
        .await
        .expect("Redis");
        let bus = RedisEventBus::new(redis, "faults-node".to_string());
        Some(Self {
            db: Some(db),
            seed,
            redis_url,
            receiver,
            config,
            http,
            bus,
            tenant,
        })
    }

    async fn events_server(&self) -> Service {
        Service::start(
            "events-server",
            BIN,
            &self.config,
            &loopback(self.http, "/health"),
        )
        .await
    }

    /// A call started, as a node announces it; its call id.
    async fn call_initiated(&self) -> String {
        let call_id = CallId::new();
        self.bus
            .publish(PlatformEvent::Call(CallEvent::Initiated {
                call_id,
                tenant_id: self.tenant,
                from: "sip:alice@faults.local".to_string(),
                to: "sip:bob@faults.local".to_string(),
                direction: CallDirection::Outbound,
                timestamp: SystemTime::now(),
                trunk_name: None,
                trunk_id: None,
                node_id: Some("faults-node".to_string()),
            }))
            .await
            .expect("published");
        call_id.as_uuid().to_string()
    }

    /// An emergency call placed; its call id as a notice names it.
    async fn emergency(&self) -> String {
        let call_id = CallId::new();
        self.bus
            .publish(PlatformEvent::Call(CallEvent::Emergency {
                call_id,
                tenant_id: self.tenant,
                from: "sip:alice@faults.local".to_string(),
                dialed: "911".to_string(),
                kind: "live".to_string(),
                callback_number: None,
                location_source: None,
                location_summary: Some("Building 1".to_string()),
                client_address: None,
                trunk_name: None,
                notify_contacts: Vec::new(),
                timestamp: SystemTime::now(),
                node_id: Some("faults-node".to_string()),
            }))
            .await
            .expect("published");
        call_id.as_uuid().to_string()
    }

    async fn finish(mut self) {
        let _ = &self.redis_url;
        let _ = &self.seed;
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

macro_rules! stage_or_skip {
    () => {
        match Stage::start().await {
            Some(s) => s,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL or FCP_TEST_REDIS_URL is not set");
                return;
            }
        }
    };
}

/// Polls `check` until it holds or `within` runs out.
async fn eventually(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if check() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Every call the test announced was delivered, each under one event id
/// however often it was sent; and every emergency call was notified.
fn assert_all_arrived(receiver: &Receiver, calls: &[String], emergencies: &[String], logs: &str) {
    let initiated = receiver.initiated();
    for call in calls {
        match initiated.get(call) {
            None => panic!("call.initiated for {call} never arrived: {initiated:#?}\n{logs}"),
            Some(ids) if ids.len() != 1 => panic!(
                "call.initiated for {call} arrived under {} event ids, so a receiver cannot tell it is one event: {ids:?}\n{logs}",
                ids.len()
            ),
            Some(_) => {}
        }
    }
    let notified = receiver.notified();
    for call in emergencies {
        assert!(
            notified.contains(call),
            "nobody was told of emergency call {call}: {notified:?}\n{logs}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_raised_while_the_events_server_is_down_are_delivered() {
    let stage = stage_or_skip!();
    // A first run makes the consumer group, as a deployment's has long
    // since; then the server is killed.
    let first = stage.events_server().await;
    let warmup = stage.call_initiated().await;
    let receiver = stage.receiver.clone();
    let delivered = eventually(A_WHILE, || receiver.initiated().contains_key(&warmup)).await;
    let logs = first.kill().await;
    assert!(delivered, "the first event never arrived\n{logs}");

    let calls = vec![
        stage.call_initiated().await,
        stage.call_initiated().await,
        stage.call_initiated().await,
    ];
    let emergencies = vec![stage.emergency().await];

    let second = stage.events_server().await;
    let receiver = stage.receiver.clone();
    let (c, e) = (calls.clone(), emergencies.clone());
    eventually(A_WHILE, || {
        let initiated = receiver.initiated();
        let notified = receiver.notified();
        c.iter().all(|x| initiated.contains_key(x)) && e.iter().all(|x| notified.contains(x))
    })
    .await;
    let logs = second.stop().await;
    assert_all_arrived(&stage.receiver, &calls, &emergencies, &logs);
    stage.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_killed_events_server_delivers_what_it_had_taken() {
    let stage = stage_or_skip!();
    let first = stage.events_server().await;

    // The receiver holds every request open, so whatever the server took
    // off the stream is in its hands, not delivered, when it is killed.
    stage.receiver.holding.store(true, Ordering::Release);
    let calls = vec![
        stage.call_initiated().await,
        stage.call_initiated().await,
        stage.call_initiated().await,
    ];
    let emergencies = vec![stage.emergency().await];
    let receiver = stage.receiver.clone();
    // Three webhooks and one notice in flight.
    let taken = eventually(A_WHILE, || receiver.held() >= 4).await;
    if !taken {
        let logs = first.kill().await;
        panic!(
            "the server did not take the events: {} requests held\n{logs}",
            stage.receiver.held()
        );
    }
    let killed_logs = first.kill().await;
    stage.receiver.holding.store(false, Ordering::Release);

    let second = stage.events_server().await;
    let receiver = stage.receiver.clone();
    let (c, e) = (calls.clone(), emergencies.clone());
    eventually(A_WHILE, || {
        let initiated = receiver.initiated();
        let notified = receiver.notified();
        c.iter().all(|x| initiated.contains_key(x)) && e.iter().all(|x| notified.contains(x))
    })
    .await;
    let logs = second.stop().await;
    assert_all_arrived(
        &stage.receiver,
        &calls,
        &emergencies,
        &format!("{killed_logs}\n{logs}"),
    );
    stage.finish().await;
}
