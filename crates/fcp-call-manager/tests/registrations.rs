//! Registration lookups kept for a TTL, over real SIP
//! (`docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 2): what a call costs in
//! round trips to the registration store once the cache holds its
//! parties, and that a cached answer never outlives a change the node can
//! know about — its own REGISTER, or another node's announced on the bus.
//!
//! Counts, not timings: a loaded CI box says nothing about latency.
//! Skipped without `FCP_TEST_DATABASE_URL`; the two-node test also wants
//! `FCP_TEST_REDIS_URL`.

use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);

/// One call manager.
struct Node {
    service: Option<Service>,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Node {
    /// A call manager on `database_url`. `registrar` is added to its
    /// `[registrar]` section; `redis`, when given, carries its events.
    async fn start(name: &str, database_url: &str, registrar: &str, redis: Option<&str>) -> Self {
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let (redis_url, events) = match redis {
            Some(url) => (
                format!("redis_url = \"{url}\""),
                "[events]\nuse_redis = true".to_string(),
            ),
            None => (String::new(), String::new()),
        };
        let config = format!(
            r#"
[service]
node_id = "{name}"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"
{registrar}

[storage]
database_url = "{database_url}"
{redis_url}

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

{events}
"#,
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
        );
        let service = Service::start(name, BIN, &config, &loopback(http, "/health")).await;
        Self {
            service: Some(service),
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            _dir: dir,
        }
    }

    async fn phone(&self, user: &SeededUser) -> Phone {
        Phone::new(&user.username, &user.password, REALM, self.sip)
            .await
            .expect("phone binds")
    }

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

    async fn status(&self) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, "/status"))
            .await
            .expect("GET /status");
        assert_eq!(code, 200, "GET /status: {body}");
        serde_json::from_str(&body).expect("status is JSON")
    }

    /// `(direct, alias, cached)` so far: the first two are round trips to
    /// the store, the last lookups the cache answered.
    async fn reads(&self) -> Reads {
        let status = self.status().await;
        let r = &status["registration_reads"];
        let n = |k: &str| r[k].as_u64().unwrap_or_default();
        Reads {
            direct: n("direct"),
            alias: n("alias"),
            cached: n("cached"),
        }
    }

    async fn stop(mut self) {
        if let Some(service) = self.service.take() {
            service.stop().await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Reads {
    direct: u64,
    alias: u64,
    cached: u64,
}

impl std::ops::Sub for Reads {
    type Output = Reads;
    fn sub(self, before: Reads) -> Reads {
        Reads {
            direct: self.direct - before.direct,
            alias: self.alias - before.alias,
            cached: self.cached - before.cached,
        }
    }
}

impl Reads {
    fn round_trips(&self) -> u64 {
        self.direct + self.alias
    }
}

/// Alice calls Bob, Bob answers, Alice hangs up: what that cost.
async fn one_call(node: &Node, alice: &Phone, bob: &SeededUser) -> Reads {
    let before = node.reads().await;
    let call = alice.call(&bob.extension).await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    node.reads().await - before
}

/// Two calls between the same two phones on a node with `registrar` in
/// its `[registrar]`: what each cost.
async fn two_calls(registrar: &str) -> Option<(Reads, Reads)> {
    let db = TestDatabase::create().await?;
    let seed = Seed::new(db.url(), REALM).await;
    let node = Node::start("e2e-registrations", db.url(), registrar, None).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = node.registered(&alice).await;
    let bob_phone = node.registered(&bob).await;
    bob_phone.auto_answer(true);

    let first = one_call(&node, &alice_phone, &bob).await;
    let second = one_call(&node, &alice_phone, &bob).await;

    node.stop().await;
    db.drop_db().await;
    Some((first, second))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_second_call_costs_no_registration_reads() {
    // Phase 1 measured two round trips per call setup. The first call
    // after the phones register still pays them — a REGISTER empties the
    // cache for its address — and the next is answered from memory.
    let Some((first, second)) = two_calls("").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    eprintln!("PHASE2 first call: {first:?}; second call: {second:?}");

    assert!(
        first.direct >= 2,
        "the first call should read both parties from the store: {first:?}"
    );
    assert_eq!(
        second.round_trips(),
        0,
        "the second call went to the store: {second:?}"
    );
    assert!(
        second.cached >= 2,
        "the second call's lookups were not answered by the cache: {second:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_aliases_the_second_call_costs_nothing_either() {
    // Phase 1: two aliases treble a call's reads, and four of the six find
    // nobody. Those empty answers are kept too, or the cache would save
    // little of what an alias costs.
    let aliases = r#"domain_aliases = ["e2e.local", "alias-one.local", "alias-two.local"]"#;
    let Some((first, second)) = two_calls(aliases).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    eprintln!("PHASE2 with two aliases, first call: {first:?}; second call: {second:?}");

    assert!(
        first.alias >= first.direct && first.alias > 0,
        "the first call should pay for every alias: {first:?}"
    );
    assert_eq!(
        second.round_trips(),
        0,
        "the second call went to the store: {second:?}"
    );
    // Every lookup the first call made — to the store, or already to the
    // cache when its INVITE was challenged and sent again — is a hit on the
    // second.
    assert_eq!(
        second.cached,
        first.round_trips() + first.cached,
        "each lookup the first call made should be a hit on the second"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_the_cache_off_every_call_pays() {
    // `cache_ttl_secs = 0` is the node as it was before phase 2.
    let Some((first, second)) = two_calls("cache_ttl_secs = 0").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    assert_eq!(first.cached + second.cached, 0);
    assert!(second.direct >= 2, "{second:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_that_moves_is_reached_where_it_now_is() {
    // Bob's binding is cached by a call. His phone then leaves and a new
    // one registers from another address, through this node: the next
    // call must ring the new one, not the address the cache last saw.
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    // A TTL no test outlives: anything seen here was the invalidation.
    let node = Node::start("e2e-moves", db.url(), "cache_ttl_secs = 600", None).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = node.registered(&alice).await;
    let old_phone = node.registered(&bob).await;
    old_phone.auto_answer(true);

    let first = one_call(&node, &alice_phone, &bob).await;
    assert!(first.direct >= 1, "{first:?}");
    // An answered call stays queued on the phone; take it, so what the
    // old phone receives from here on is only what the cache sent it.
    assert!(old_phone.incoming(SOON).await.is_some());

    assert_eq!(old_phone.register_for(0).await.unwrap(), 200);
    let new_phone = node.registered(&bob).await;
    new_phone.auto_answer(true);

    let before = node.reads().await;
    let call = alice_phone.call(&bob.extension).await.unwrap();
    call.answered(SOON)
        .await
        .expect("the call reaches Bob's new phone");
    assert_eq!(call.hangup().await.unwrap(), 200);
    let moved = node.reads().await - before;
    assert!(
        old_phone
            .incoming(Duration::from_millis(300))
            .await
            .is_none(),
        "the old phone was rung from a cached binding"
    );
    assert!(
        moved.direct >= 1,
        "Bob's REGISTER should have sent the next lookup to the store: {moved:?}"
    );

    node.stop().await;
    db.drop_db().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_nodes_register_is_seen_when_its_event_arrives() {
    // Two call managers on one database, their events on one Redis. Node
    // B tries Bob while he is registered nowhere and keeps the empty
    // answer. Bob then registers at node A. Node B must find him on its
    // next call — long before the TTL — because A's registration event
    // told it to forget.
    let Some(redis) = private_redis_url() else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let ttl = "cache_ttl_secs = 600";
    let node_a = Node::start("e2e-reg-a", db.url(), ttl, Some(&redis)).await;
    let node_b = Node::start("e2e-reg-b", db.url(), ttl, Some(&redis)).await;
    let alice = seed.user("alice", "alice-pw", "1001").await;
    let bob = seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = node_b.registered(&alice).await;

    let before = node_b.reads().await;
    let call = alice_phone.call(&bob.extension).await.unwrap();
    call.answered(SOON)
        .await
        .expect_err("Bob is registered nowhere yet");
    let missed = node_b.reads().await - before;
    assert!(missed.direct >= 1, "{missed:?}");

    let bob_phone = node_a.registered(&bob).await;
    bob_phone.auto_answer(true);

    // The event crosses Redis on its own time; a call that still finds
    // nobody re-caches nothing the event will not clear, so try until it
    // lands. Well inside the TTL, so a success here is the event's doing.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let call = alice_phone.call(&bob.extension).await.unwrap();
        if call.answered(SOON).await.is_ok() {
            assert_eq!(call.hangup().await.unwrap(), 200);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "node B never learned that Bob registered at node A"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    node_a.stop().await;
    node_b.stop().await;
    db.drop_db().await;
}

/// A suspended user's phone is refused; a Locked user's — the web
/// sign-in's lockout — keeps working (`docs/IDENTITY_AND_DEVICES.md`
/// phase 5). Each status is set before the phone's first REGISTER, so the
/// node's credential cache holds nothing older.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_suspended_users_phone_is_refused_and_a_locked_ones_is_not() {
    use fcp_core::storage::CredentialStore;
    let Some(db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    let node = Node::start("e2e-suspended", db.url(), "", None).await;
    let sus = seed.user("sus", "sus-pw", "1101").await;
    let lok = seed.user("lok", "lok-pw", "1102").await;
    for (name, status) in [
        ("sus", fcp_core::AccountStatus::Suspended),
        ("lok", fcp_core::AccountStatus::Locked),
    ] {
        let mut user = seed
            .storage()
            .get_user_in_tenant(name, &seed.tenant_id)
            .await
            .unwrap()
            .unwrap();
        user.account_status = status;
        seed.storage().put_user(user).await.unwrap();
    }

    let refused = node.phone(&sus).await.register().await.unwrap();
    assert_ne!(refused, 200, "a suspended user's phone registered");
    assert_eq!(
        node.phone(&lok).await.register().await.unwrap(),
        200,
        "a locked user's phone was refused"
    );

    node.stop().await;
    db.drop_db().await;
}
