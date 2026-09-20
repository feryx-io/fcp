//! What this node will carry, over real SIP
//! (`docs/CAPACITY_AND_SURVIVABILITY.md` §11, T6): the node's own limit and
//! the tenant's refuse the call past them with a 503, the place a call held
//! is given back when it ends, a registration flood is refused before it
//! reaches the store, and the node says what it allows and what it carries.
//! Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// Long enough for the node's backstop read of the tenants to happen twice.
const REFRESHED: Duration = Duration::from_secs(4);

/// A running call manager with limits of its own.
struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database to test against. `admission` is the
    /// node's `[admission]` section, written by the test.
    async fn start(admission: &str) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "e2e-capacity"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 5

[b2bua.forking]
timeout = 3

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "info"
format = "compact"

# A tenant's limit reaches the node on the same read as its trunks, and
# nothing here publishes a change, so the backstop is turned right down.
[trunk_routing]
refresh_secs = 1

{admission}
"#,
            db = db.url(),
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            service: tokio::sync::Mutex::new(Some(service)),
            db: Some(db),
            seed,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            _dir: dir,
        })
    }

    async fn phone(&self, user: &SeededUser) -> Phone {
        Phone::new(&user.username, &user.password, REALM, self.sip)
            .await
            .expect("phone binds")
    }

    /// A registered phone for `user`.
    async fn registered(&self, user: &SeededUser) -> Phone {
        let phone = self.phone(user).await;
        assert_eq!(
            phone.register().await.expect("REGISTER is answered"),
            200,
            "{} could not register",
            user.username
        );
        phone
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(code, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
    }

    /// The node's own account of what it admitted and refused.
    async fn admission(&self) -> serde_json::Value {
        self.get("/status").await["admission"].clone()
    }

    /// `/metrics` as text.
    async fn metrics(&self) -> String {
        let (code, body) = http_get(&loopback(self.http, "/metrics"))
            .await
            .expect("GET /metrics");
        assert_eq!(code, 200, "GET /metrics: {body}");
        body
    }

    /// Give up with the service's logs.
    async fn fail(&self, why: &str) -> ! {
        let logs = match self.service.lock().await.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        panic!("{why}\n{logs}");
    }

    async fn finish(mut self) {
        if let Some(service) = self.service.lock().await.take() {
            service.stop().await;
        }
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

macro_rules! stack_or_skip {
    ($admission:expr) => {
        match Stack::start($admission).await {
            Some(s) => s,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// A node at its concurrent-call limit refuses the next call with a 503,
/// and takes it once a call has ended — the place a call holds is given
/// back however the call goes away.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_carries_only_what_it_allows() {
    let stack = stack_or_skip!("[admission]\nmax_concurrent_calls = 1");
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let dave = stack.seed.user("dave", "dave-pw", "1004").await;

    let alice_phone = stack.registered(&alice).await;
    let bob_phone = stack.registered(&bob).await;
    let carol_phone = stack.registered(&carol).await;
    let dave_phone = stack.registered(&dave).await;

    // The one call this node allows.
    let first = alice_phone.call(&bob.extension).await.unwrap();
    let incoming = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    let answered = incoming.answer().await.expect("Bob answers");
    first.answered(SOON).await.expect("Alice hears Bob");

    // The next one is refused, and Dave is never rung for it.
    let refused = carol_phone.call(&dave.extension).await.unwrap();
    match refused.failed(SOON).await {
        Some(503) => {}
        other => {
            stack
                .fail(&format!("a call past the node's limit gave {other:?}"))
                .await
        }
    }
    assert!(
        dave_phone.incoming(Duration::from_secs(1)).await.is_none(),
        "Dave was rung for a call the node had refused"
    );
    let admission = stack.admission().await;
    assert_eq!(admission["refused"]["node_calls"], 1, "{admission}");
    assert_eq!(admission["concurrent_calls"], 1, "{admission}");

    // The place is given back when the call ends.
    answered.hangup().await.expect("Bob hangs up");
    assert!(first.ended(SOON).await, "Alice's call ends");
    let freed = tokio::time::Instant::now() + SOON;
    loop {
        if stack.admission().await["concurrent_calls"] == 0 {
            break;
        }
        if tokio::time::Instant::now() > freed {
            let admission = stack.admission().await;
            stack
                .fail(&format!("the ended call still counts: {admission}"))
                .await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let again = carol_phone.call(&dave.extension).await.unwrap();
    let incoming = dave_phone
        .incoming(SOON)
        .await
        .expect("Dave's phone rings once there is room");
    incoming.answer().await.expect("Dave answers");
    again.answered(SOON).await.expect("Carol hears Dave");

    stack.finish().await;
}

/// A tenant's own `max_concurrent_calls` is a contract, not a capacity:
/// the node has room, and the call is still refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenants_limit_refuses_its_own_call() {
    let stack = stack_or_skip!("[admission]");
    stack
        .seed
        .tenant_call_limit(stack.seed.tenant_id, Some(1))
        .await;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let dave = stack.seed.user("dave", "dave-pw", "1004").await;
    // The limit reaches the node on its next read of the tenants.
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.registered(&alice).await;
    let bob_phone = stack.registered(&bob).await;
    let carol_phone = stack.registered(&carol).await;
    let dave_phone = stack.registered(&dave).await;

    let first = alice_phone.call(&bob.extension).await.unwrap();
    let incoming = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    incoming.answer().await.expect("Bob answers");
    first.answered(SOON).await.expect("Alice hears Bob");

    let refused = carol_phone.call(&dave.extension).await.unwrap();
    match refused.failed(SOON).await {
        Some(503) => {}
        other => {
            stack
                .fail(&format!("a call past the tenant's limit gave {other:?}"))
                .await
        }
    }
    assert!(
        dave_phone.incoming(Duration::from_secs(1)).await.is_none(),
        "Dave was rung for a call the tenant's limit had refused"
    );

    // Refused for the tenant, not for the node, which allows everything.
    let admission = stack.admission().await;
    assert_eq!(admission["refused"]["tenant_calls"], 1, "{admission}");
    assert_eq!(admission["refused"]["node_calls"], 0, "{admission}");
    assert_eq!(
        admission["limits"]["max_concurrent_calls"], 0,
        "{admission}"
    );
    let tenants = admission["tenants"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        tenants
            .iter()
            .find(|t| t["tenant_id"] == stack.seed.tenant_id.to_string())
            .map(|t| t["concurrent_calls"].clone()),
        Some(serde_json::json!(1)),
        "{admission}"
    );

    stack.finish().await;
}

/// A device that registers over and over is refused before its REGISTER
/// reaches the store.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_registration_flood_is_refused() {
    // Four REGISTERs a second from one address: two registrations, since
    // each is challenged and answered.
    let stack = stack_or_skip!("[admission]\nmax_register_cps_per_source = 4");
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let phone = stack.phone(&alice).await;

    let mut codes = Vec::new();
    for _ in 0..6 {
        codes.push(phone.register().await.expect("REGISTER is answered"));
    }
    assert!(
        codes.contains(&503),
        "a flood from one address was never refused: {codes:?}"
    );

    let admission = stack.admission().await;
    assert!(
        admission["refused"]["register_rate"].as_u64().unwrap_or(0) > 0,
        "{admission}"
    );

    // And the limit is a rate, not a ban: a second later the device is
    // registered again.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        phone.register().await.expect("REGISTER is answered"),
        200,
        "the device was still refused a second later"
    );

    stack.finish().await;
}

/// The node says what it allows and what it is carrying, in `/status` and
/// in `/metrics`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_says_what_it_allows() {
    let stack = stack_or_skip!(
        "[admission]\nmax_cps = 30\nmax_concurrent_calls = 200\nmax_concurrent_registrations = 400"
    );
    let admission = stack.admission().await;
    assert_eq!(admission["limits"]["max_cps"], 30, "{admission}");
    assert_eq!(
        admission["limits"]["max_concurrent_calls"], 200,
        "{admission}"
    );
    assert_eq!(
        admission["limits"]["max_concurrent_registrations"], 400,
        "{admission}"
    );

    let metrics = stack.metrics().await;
    for series in [
        "fcp_admission_concurrent_calls{node=\"e2e-capacity\"}",
        "fcp_admission_current_cps{node=\"e2e-capacity\"}",
        "fcp_admission_concurrent_registrations{node=\"e2e-capacity\"}",
        "fcp_admission_capacity{node=\"e2e-capacity\",limit=\"max_cps\"} 30",
    ] {
        assert!(metrics.contains(series), "{series} missing from /metrics");
    }

    stack.finish().await;
}
