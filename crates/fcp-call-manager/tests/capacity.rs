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
    /// node's `[admission]` section and `service` is added to its
    /// `[service]` one — a second `[service]` table would not parse.
    async fn start_with(service: &str, admission: &str) -> Option<Self> {
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
{service}

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

    /// `GET /ready`: the code and the body.
    async fn ready(&self) -> (u16, serde_json::Value) {
        let (code, body) = http_get(&loopback(self.http, "/ready"))
            .await
            .expect("GET /ready");
        (
            code,
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    /// `POST` a JSON body: the status and the answer.
    async fn post(&self, path: &str, body: &str) -> (u16, serde_json::Value) {
        let (code, text) = fcp_testkit::smoke::http_post(&loopback(self.http, path), body)
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        (
            code,
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        )
    }

    /// `DELETE` a path: the status and the answer.
    async fn delete(&self, path: &str) -> (u16, serde_json::Value) {
        let (code, text) =
            fcp_testkit::smoke::http_request("DELETE", &loopback(self.http, path), &[], None)
                .await
                .unwrap_or_else(|e| panic!("DELETE {path}: {e}"));
        (
            code,
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
        )
    }

    /// The records at `/cdrs`, once at least `n` are there.
    async fn cdrs(&self, n: usize, within: Duration) -> Vec<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let body = self.get("/cdrs").await;
            let cdrs = body["cdrs"].as_array().cloned().unwrap_or_default();
            if cdrs.len() >= n {
                return cdrs;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {n} CDRs after {within:?}: {body}",
                cdrs.len()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
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
        stack_or_skip!("", $admission)
    };
    ($service:expr, $admission:expr) => {
        match Stack::start_with($service, $admission).await {
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

    // Six devices at once, twelve REGISTERs: a flood is concurrent, and a
    // burst sent one after another would spread over as many seconds as a
    // slow machine takes and never reach the limit at all.
    let mut phones = Vec::new();
    for _ in 0..6 {
        phones.push(stack.phone(&alice).await);
    }
    let answers = tokio::join!(
        phones[0].register(),
        phones[1].register(),
        phones[2].register(),
        phones[3].register(),
        phones[4].register(),
        phones[5].register(),
    );
    let codes = [
        answers.0, answers.1, answers.2, answers.3, answers.4, answers.5,
    ]
    .map(|answer| answer.expect("REGISTER is answered"));
    assert!(
        codes.contains(&503),
        "a flood from one address was never refused: {codes:?}"
    );

    let admission = stack.admission().await;
    assert!(
        admission["refused"]["register_rate"].as_u64().unwrap_or(0) > 0,
        "{admission}"
    );

    // And the limit is a rate, not a ban: once the flood is over, a
    // device registers again.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        phones[0].register().await.expect("REGISTER is answered"),
        200,
        "the device was still refused once the flood was over"
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

/// A draining node keeps the calls it is carrying and refuses new ones,
/// and a drain called off leaves it serving again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draining_node_keeps_its_calls_and_takes_no_new_one() {
    // Long enough that the deadline never arrives during the test.
    let stack = stack_or_skip!("drain_wait_secs = 300", "");
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let dave = stack.seed.user("dave", "dave-pw", "1004").await;
    let alice_phone = stack.registered(&alice).await;
    let bob_phone = stack.registered(&bob).await;
    let carol_phone = stack.registered(&carol).await;
    let dave_phone = stack.registered(&dave).await;

    let established = alice_phone.call(&bob.extension).await.unwrap();
    let incoming = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    let answered = incoming.answer().await.expect("Bob answers");
    established.answered(SOON).await.expect("Alice hears Bob");

    assert_eq!(stack.ready().await.0, 200, "serving before the drain");
    let (code, drain) = stack.post("/drain", "{}").await;
    assert_eq!(code, 200, "{drain}");
    assert_eq!(drain["status"], "draining", "{drain}");

    // A load balancer is told to stop choosing this node; a liveness
    // probe still sees a healthy process.
    let (ready_code, ready) = stack.ready().await;
    assert_eq!(ready_code, 503, "{ready}");
    assert_eq!(ready["draining"], true, "{ready}");
    assert_eq!(stack.get("/health").await["status"], "healthy");
    assert_eq!(stack.get("/status").await["draining"], true);

    // New calls are refused, and the callee is never rung.
    let refused = carol_phone.call(&dave.extension).await.unwrap();
    match refused.failed(SOON).await {
        Some(503) => {}
        other => {
            stack
                .fail(&format!("a call to a draining node gave {other:?}"))
                .await
        }
    }
    assert!(
        dave_phone.incoming(Duration::from_secs(1)).await.is_none(),
        "Dave was rung by a node that was draining"
    );
    assert_eq!(stack.admission().await["refused"]["draining"], 1);

    // The call already up is untouched: no BYE, and both ends still there.
    assert!(
        !established.ended(Duration::from_secs(2)).await,
        "a drain hung up a call it should have waited for"
    );

    // A device that registers while the node drains is kept on a short
    // leash, so it comes back and is placed somewhere that is serving.
    let eve = stack.seed.user("eve", "eve-pw", "1005").await;
    let eve_phone = stack.registered(&eve).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let registrations = stack.get("/registrations").await;
    let eve_expiry = registrations["registrations"]
        .as_array()
        .expect("registrations")
        .iter()
        .find(|r| r["aor"].as_str().is_some_and(|aor| aor.contains("eve")))
        .and_then(|r| r["expires_at_epoch"].as_u64())
        .unwrap_or_else(|| panic!("eve is not registered: {registrations}"));
    assert!(
        eve_expiry <= now + 70,
        "a draining node granted {}s, not the 60 it caps at: {registrations}",
        eve_expiry.saturating_sub(now)
    );
    drop(eve_phone);

    // Called off: the node serves again, and the call it kept is still up.
    let (code, served) = stack.delete("/drain").await;
    assert_eq!(code, 200, "{served}");
    assert_eq!(served["status"], "serving", "{served}");
    assert_eq!(stack.ready().await.0, 200, "serving after the drain is off");

    let again = carol_phone.call(&dave.extension).await.unwrap();
    let incoming = dave_phone
        .incoming(SOON)
        .await
        .expect("Dave's phone rings once the node serves again");
    incoming.answer().await.expect("Dave answers");
    again.answered(SOON).await.expect("Carol hears Dave");
    assert!(
        !established.ended(Duration::from_millis(500)).await,
        "the call kept through the drain was dropped"
    );

    // Both calls hang up before the node does: this test is about a drain
    // that was called off, not about what a shutdown does to what is left.
    answered.hangup().await.expect("Bob hangs up");
    again.hangup().await.expect("Carol hangs up");
    stack.finish().await;
}

/// A call still up when the drain deadline comes is ended with a BYE that
/// says why, and its record says the node was draining.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drain_ends_the_calls_left_at_its_deadline() {
    let stack = stack_or_skip!("drain_wait_secs = 2", "");
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.registered(&alice).await;
    let bob_phone = stack.registered(&bob).await;

    let call = alice_phone.call(&bob.extension).await.unwrap();
    let incoming = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    let bob_call = incoming.answer().await.expect("Bob answers");
    call.answered(SOON).await.expect("Alice hears Bob");

    let (code, _) = stack.post("/drain", "{}").await;
    assert_eq!(code, 200);

    // Two seconds of grace, then both ends are hung up.
    assert!(
        call.ended(SOON).await,
        "the caller was not hung up at the drain deadline"
    );
    assert!(
        bob_call.ended(SOON).await,
        "the callee was not hung up at the drain deadline"
    );

    let cdrs = stack.cdrs(1, SOON).await;
    let cdr = &cdrs[0];
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert_eq!(cdr["hangup_party"], "System", "{cdr}");
    assert_eq!(cdr["hangup_cause"], "NormalClearing", "{cdr}");
    assert_eq!(cdr["sip_reason"], "Node draining", "{cdr}");

    // And the node is still there, drained: a drain is not a shutdown.
    assert_eq!(stack.ready().await.0, 503);
    assert_eq!(stack.get("/status").await["active_calls"], 0);

    stack.finish().await;
}
