//! Faults with calls up (`docs/DEEP_TESTING.md` D5): Postgres and Redis
//! taken away from a running call manager, refused or silent, and brought
//! back; a call manager killed with `SIGKILL` while it carries a call and
//! started again. After each, what lived, what the phones heard, whether
//! registrations, records and events caught up, and that nothing needed a
//! restart by hand.
//!
//! The node reaches its stores through [`CutLink`]s, so a fault is the
//! network's: a refused store resets every connection and refuses new
//! ones, a silent one holds them open and answers nothing.
//!
//! The primary failed over to its standby (D5c) runs its own pair of
//! Postgres servers ([`PgPair`]), with the node given an address that
//! moves to the promoted standby, as a VIP or a DNS name does.
//!
//! Skipped without `FCP_TEST_DATABASE_URL`; the Redis faults and the
//! killed node also need `FCP_TEST_REDIS_URL`, and the failover the
//! Postgres server's binaries.

use fcp_core::calling::CallingSettings;
use fcp_core::storage::CdrKind;
use fcp_testkit::cluster::private_redis_url;
use fcp_testkit::cut::CutLink;
use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::pg_pair::PgPair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Call, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// How long a SIP request may go unanswered while a store is away. A
/// phone gives up on a request after 32 s (Timer F); the node must answer
/// well before, with something a phone can act on.
const ANSWERED_WITHIN: Duration = Duration::from_secs(8);
/// How long the node has, once a store is back, to be whole again.
const CAUGHT_UP: Duration = Duration::from_secs(15);
/// The cluster lease time the nodes run with (`[cluster] lease_ttl_secs`).
const LEASE_TTL_SECS: u64 = 8;

/// Which way a store goes away.
#[derive(Clone, Copy, Debug)]
enum Fault {
    /// Connections reset, new ones refused: the store's process gone.
    Refused,
    /// Connections held, nothing answered: a partition.
    Silent,
}

impl Fault {
    fn apply(self, link: &CutLink) {
        match self {
            Fault::Refused => link.cut(),
            Fault::Silent => link.silence(),
        }
    }
}

/// One call manager.
struct Node {
    service: Option<Service>,
    sip: SocketAddr,
    http: u16,
}

impl Node {
    /// A call manager named `name` on `db`, in a cluster on `redis` if
    /// given. Its CDR spool is under `dir`, named for it, so the same
    /// node started again finds what it left.
    async fn start(name: &str, db: &str, redis: Option<&str>, dir: &Path) -> Self {
        Self::start_with(name, db, redis, dir, "").await
    }

    /// As [`start`](Self::start), with more `[storage]` keys.
    async fn start_with(
        name: &str,
        db: &str,
        redis: Option<&str>,
        dir: &Path,
        storage: &str,
    ) -> Self {
        let (sip, http) = (free_port(), free_port());
        Self {
            service: Some(start_service(name, db, redis, dir, sip, http, storage).await),
            sip: SocketAddr::from(([127, 0, 0, 1], sip)),
            http,
        }
    }

    async fn restart(&mut self, name: &str, db: &str, redis: Option<&str>, dir: &Path) {
        assert!(self.service.is_none(), "{name} is still running");
        self.service =
            Some(start_service(name, db, redis, dir, self.sip.port(), self.http, "").await);
    }

    async fn kill(&mut self) -> String {
        self.service.take().expect("running").kill().await
    }

    /// `GET /ready`: its code and body.
    async fn ready(&self) -> (u16, serde_json::Value) {
        let (code, body) = http_get(&loopback(self.http, "/ready"))
            .await
            .expect("GET /ready");
        (code, serde_json::from_str(&body).unwrap_or_default())
    }

    async fn status(&self) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, "/status"))
            .await
            .expect("GET /status");
        assert_eq!(code, 200, "GET /status: {body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn logs(&mut self) -> String {
        match self.service.take() {
            Some(s) => s.kill().await,
            None => String::new(),
        }
    }

    async fn finish(&mut self) {
        if let Some(s) = self.service.take() {
            s.stop().await;
        }
    }
}

async fn start_service(
    name: &str,
    db: &str,
    redis: Option<&str>,
    dir: &Path,
    sip: u16,
    http: u16,
    storage: &str,
) -> Service {
    let rtp_start = allocate_rtp_range(40);
    let cluster = redis
        .map(|url| {
            format!(
                "redis_url = \"{url}\"\n\n[cluster]\nenabled = true\n\
                 heartbeat_interval_secs = 1\nheartbeat_ttl_secs = 3\n\
                 lease_ttl_secs = {LEASE_TTL_SECS}\n\n\
                 [events]\nuse_redis = true"
            )
        })
        .unwrap_or_default();
    let config = format!(
        r#"
[service]
node_id = "{name}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"
cache_ttl_secs = 0

[cdr]
spool_dir = "{dir}/{name}-cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 30

[b2bua.forking]
timeout = 10

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/{name}-recordings"

[logging]
level = "info"
format = "compact"

[storage]
database_url = "{db}"
{storage}
{cluster}
"#,
        dir = dir.display(),
        rtp_end = rtp_start + 40,
    );
    Service::start(name, BIN, &config, &loopback(http, "/health")).await
}

/// A database, its seed, and the users every test calls with.
struct Cast {
    db: Option<TestDatabase>,
    seed: Seed,
    alice: SeededUser,
    bob: SeededUser,
    carol: SeededUser,
    dave: SeededUser,
    dir: tempfile::TempDir,
}

impl Cast {
    async fn new() -> Option<Self> {
        let db = TestDatabase::create().await?;
        let mut cast = Self::on(db.url()).await;
        cast.db = Some(db);
        Some(cast)
    }

    /// The cast in a database the test made some other way.
    async fn on(url: &str) -> Self {
        let seed = Seed::new(url, REALM).await;
        let alice = seed.user("alice", "alice-pw", "1001").await;
        let bob = seed.user("bob", "bob-pw", "1002").await;
        let carol = seed.user("carol", "carol-pw", "1003").await;
        let dave = seed.user("dave", "dave-pw", "1004").await;
        Self {
            db: None,
            seed,
            alice,
            bob,
            carol,
            dave,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn db_url(&self) -> &str {
        self.db.as_ref().unwrap().url()
    }

    /// Bob takes one call at a time: a second is busy, so whether the
    /// node thinks he is on a call is something a caller hears.
    async fn bob_without_call_waiting(&self) {
        let storage = self.seed.storage();
        let row = fcp_core::storage::CredentialStore::get_user_in_tenant(
            storage,
            &self.bob.username,
            &self.seed.tenant_id,
        )
        .await
        .unwrap()
        .unwrap();
        let mut settings = CallingSettings::defaults(self.seed.tenant_id, row.id);
        settings.call_waiting = false;
        fcp_core::CallingStore::put_calling_settings(storage, settings)
            .await
            .unwrap();
    }

    async fn phone(&self, user: &SeededUser, node: &Node) -> Phone {
        let phone = Phone::new(&user.username, &user.password, REALM, node.sip)
            .await
            .unwrap();
        assert_eq!(
            phone.register().await.unwrap(),
            200,
            "{} registers",
            user.username
        );
        phone
    }

    /// The records of calls in the database, once there are `n`.
    async fn calls_recorded(&self, n: usize, within: Duration) -> usize {
        self.seed.cdrs(Some(CdrKind::Call), n, within).await.len()
    }

    async fn finish(mut self) {
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

macro_rules! cast_or_skip {
    () => {
        match Cast::new().await {
            Some(c) => c,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// A's call to B, answered, heard both ways.
async fn connected(a: &Phone, b: &Phone, target: &str) -> (Call, Call) {
    let call = a.call(target).await.unwrap();
    let incoming = b.incoming(SOON).await.expect("the callee rings");
    let answered = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    both_ways(&call, &answered, "as it is answered").await;
    (call, answered)
}

async fn both_ways(a: &Call, b: &Call, when: &str) {
    a.media().play_tone(440.0, 2000);
    assert!(
        b.media().heard(440, SOON).await,
        "the callee hears the caller {when} ({} packets)",
        b.media().packets_received()
    );
    b.media().play_tone(880.0, 2000);
    assert!(
        a.media().heard(880, SOON).await,
        "the caller hears the callee {when} ({} packets)",
        a.media().packets_received()
    );
}

/// The caller hangs up and the callee is told.
async fn hung_up(caller_call: &Call, callee: &Phone, when: &str) {
    assert_eq!(caller_call.hangup().await.unwrap(), 200, "BYE {when}");
    assert!(
        callee
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "the callee gets the BYE {when}"
    );
}

/// A REGISTER's final answer and how long it took.
async fn register_timed(phone: &Phone) -> (Option<u16>, Duration) {
    let started = Instant::now();
    let code = tokio::time::timeout(Duration::from_secs(40), phone.register())
        .await
        .ok()
        .and_then(Result::ok);
    (code, started.elapsed())
}

/// How a new call between two registered phones went: answered, refused
/// with a code, or nothing (`None`) in `within`.
async fn try_call(
    from: &Phone,
    to: &Phone,
    target: &str,
    within: Duration,
) -> Result<Call, Option<u16>> {
    let call = from.call(target).await.map_err(|_| None)?;
    let deadline = Instant::now() + within;
    loop {
        if let Some(incoming) = to.incoming(Duration::from_millis(200)).await {
            let answered = incoming.answer().await.map_err(|_| None)?;
            call.answered(SOON).await.map_err(|_| None)?;
            let _ = answered;
            return Ok(call);
        }
        if let Some(code) = call.failed(Duration::from_millis(10)).await {
            return Err(Some(code));
        }
        if Instant::now() > deadline {
            return Err(None);
        }
    }
}

/// Postgres goes away while Alice and Bob talk, with Carol and Dave
/// registered.
///
/// - The call lives: media both ways, and its BYE crosses.
/// - A REGISTER and a new call are answered, one way or the other,
///   within [`ANSWERED_WITHIN`], not left to the phone's timer.
/// - Once Postgres is back, the call's record is written, Carol
///   registers and calls Dave, without anyone restarting the node.
async fn postgres_away(fault: Fault) {
    let cast = cast_or_skip!();
    let (link, db) = CutLink::for_url(cast.db_url()).await;
    let mut node = Node::start("e2e-faults", &db, None, cast.dir.path()).await;
    let alice = cast.phone(&cast.alice, &node).await;
    let bob = cast.phone(&cast.bob, &node).await;
    let carol = cast.phone(&cast.carol, &node).await;
    let dave = cast.phone(&cast.dave, &node).await;
    let (call, answered) = connected(&alice, &bob, &cast.bob.extension).await;

    fault.apply(&link);
    tokio::time::sleep(Duration::from_secs(1)).await;
    both_ways(&call, &answered, &format!("with Postgres {fault:?}")).await;

    let (code, took) = register_timed(&carol).await;
    let away_register = format!("{code:?} in {took:?}");
    let away_call = match tokio::time::timeout(
        Duration::from_secs(40),
        try_call(&carol, &dave, &cast.dave.extension, Duration::from_secs(35)),
    )
    .await
    {
        Ok(Ok(c)) => {
            let _ = c.hangup().await;
            "answered".to_string()
        }
        Ok(Err(code)) => format!("{code:?}"),
        Err(_) => "no outcome".to_string(),
    };
    eprintln!("Postgres {fault:?}: REGISTER {away_register}; Carol to Dave: {away_call}");
    let (ready_away, ready_body) = node.ready().await;
    hung_up(&call, &bob, &format!("with Postgres {fault:?}")).await;

    link.restore().await;
    let recorded = cast.calls_recorded(1, CAUGHT_UP).await;
    let deadline = Instant::now() + CAUGHT_UP;
    let back = loop {
        match register_timed(&carol).await {
            (Some(200), _) => break true,
            _ if Instant::now() > deadline => break false,
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    };
    if !back {
        let logs = node.logs().await;
        panic!("Carol cannot register after Postgres came back\n{logs}");
    }
    let after = match try_call(&carol, &dave, &cast.dave.extension, SOON).await {
        Ok(call) => call,
        Err(code) => {
            let logs = node.logs().await;
            panic!("Carol cannot call Dave after Postgres came back: {code:?}\n{logs}");
        }
    };
    after.hangup().await.unwrap();

    // While it was away: a 503 the phone can act on, for the REGISTER
    // and the call alike, and a node that said it was not ready.
    if took > ANSWERED_WITHIN || code != Some(503) || away_call != "Some(503)" || ready_away != 503
    {
        let logs = node.logs().await;
        panic!(
            "with Postgres {fault:?}, a REGISTER was answered {away_register}; \
             Carol to Dave: {away_call}; /ready {ready_away} {ready_body}\n{logs}"
        );
    }
    assert_eq!(ready_body["database"]["up"], false, "{ready_body}");
    assert!(recorded >= 1);
    let (ready_back, body) = node.ready().await;
    assert_eq!(ready_back, 200, "ready once Postgres is back: {body}");
    assert_eq!(node.status().await["database"]["outages"], 1);
    node.finish().await;
    cast.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_refused_mid_call() {
    postgres_away(Fault::Refused).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_silent_mid_call() {
    postgres_away(Fault::Silent).await;
}

/// How many calls' ends are in the event stream of the Redis at `redis`.
async fn calls_ended(redis: &str) -> usize {
    fcp_testkit::cluster::streamed_events(redis)
        .await
        .iter()
        .filter(|e| e["event_type"] == "call" && e["type"] == "ended")
        .count()
}

/// Redis goes away while Alice talks to Bob, who takes one call at a
/// time.
///
/// - The call lives, and its BYE crosses.
/// - A new call (Carol to Dave) is put through: Redis is never needed
///   to place a call.
/// - Once Redis is back: the node is in the cluster again, the events
///   raised while it was away are in the stream, and Bob — whose call
///   ended while his lease could not be given back — can be called.
async fn redis_away(fault: Fault) {
    let cast = cast_or_skip!();
    let Some(redis) = private_redis_url() else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    cast.bob_without_call_waiting().await;
    let (link, through) = CutLink::for_url(&redis).await;
    let mut node = Node::start("e2e-faults", cast.db_url(), Some(&through), cast.dir.path()).await;
    let alice = cast.phone(&cast.alice, &node).await;
    let bob = cast.phone(&cast.bob, &node).await;
    let carol = cast.phone(&cast.carol, &node).await;
    let dave = cast.phone(&cast.dave, &node).await;
    let (call, answered) = connected(&alice, &bob, &cast.bob.extension).await;
    let ended_before = calls_ended(&redis).await;

    fault.apply(&link);
    tokio::time::sleep(Duration::from_secs(1)).await;
    both_ways(&call, &answered, &format!("with Redis {fault:?}")).await;
    let away = tokio::time::timeout(
        Duration::from_secs(40),
        try_call(&carol, &dave, &cast.dave.extension, ANSWERED_WITHIN),
    )
    .await;
    let away = match away {
        Ok(Ok(c)) => {
            c.hangup().await.unwrap();
            Ok(())
        }
        Ok(Err(code)) => Err(format!("{code:?}")),
        Err(_) => Err("no outcome".into()),
    };
    hung_up(&call, &bob, &format!("with Redis {fault:?}")).await;
    if let Err(why) = away {
        let logs = node.logs().await;
        panic!("with Redis {fault:?}, Carol could not call Dave: {why}\n{logs}");
    }

    link.restore().await;
    let deadline = Instant::now() + CAUGHT_UP;
    loop {
        let heartbeat = fcp_testkit::cluster::heartbeat(&redis, "e2e-faults").await;
        if heartbeat.is_some() {
            break;
        }
        if Instant::now() > deadline {
            let logs = node.logs().await;
            panic!("no heartbeat {CAUGHT_UP:?} after Redis came back\n{logs}");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Bob's call is over; the cluster must not still count it.
    let restored = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(LEASE_TTL_SECS + 5);
    let bob_again = loop {
        match try_call(&alice, &bob, &cast.bob.extension, SOON).await {
            Ok(c) => {
                c.hangup().await.unwrap();
                eprintln!(
                    "Redis {fault:?}: Bob callable {:?} after Redis came back",
                    restored.elapsed()
                );
                break Ok(());
            }
            Err(code) if Instant::now() > deadline => break Err(code),
            Err(code) => {
                eprintln!("Redis {fault:?}: Bob answers {code:?}");
                tokio::time::sleep(Duration::from_secs(1)).await
            }
        }
    };
    if let Err(code) = bob_again {
        let status = node.status().await;
        let logs = node.logs().await;
        panic!(
            "Bob is still on a call {LEASE_TTL_SECS} s after Redis came back \
             (last answer {code:?})\n{status}\n{logs}"
        );
    }

    // What happened while Redis was away reached the stream: Alice to
    // Bob and Carol to Dave ended then, and Alice to Bob again since.
    let deadline = Instant::now() + CAUGHT_UP;
    loop {
        let ended = calls_ended(&redis).await - ended_before;
        if ended >= 3 {
            eprintln!("Redis {fault:?}: {ended} calls' ends in the stream");
            break;
        }
        if Instant::now() > deadline {
            let logs = node.logs().await;
            panic!("{ended} of 3 calls' ends reached the stream\n{logs}");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    node.finish().await;
    cast.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redis_refused_mid_call() {
    redis_away(Fault::Refused).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redis_silent_mid_call() {
    redis_away(Fault::Silent).await;
}

/// A call manager is killed with `SIGKILL` while it carries Alice's call
/// to Bob, who takes one call at a time; another node carries on.
///
/// - The other node puts Carol through to Bob within a lease time: the
///   dead node's call does not keep Bob busy.
/// - The dead node leaves the cluster within its heartbeat time.
/// - A record the dead node had spooled but not written (its Postgres
///   was away) is written when it starts again on the same spool.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_killed_node() {
    let cast = cast_or_skip!();
    let Some(redis) = private_redis_url() else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    cast.bob_without_call_waiting().await;
    let (a_db, a_url) = CutLink::for_url(cast.db_url()).await;
    let dir = cast.dir.path();
    let mut a = Node::start("e2e-fault-a", &a_url, Some(&redis), dir).await;
    let mut b = Node::start("e2e-fault-b", cast.db_url(), Some(&redis), dir).await;
    let alice = cast.phone(&cast.alice, &a).await;
    let bob = cast.phone(&cast.bob, &a).await;
    let carol = cast.phone(&cast.carol, &b).await;
    let dave = cast.phone(&cast.dave, &a).await;

    // Alice's call to Bob, which dies with A; and Dave's to Alice, which
    // ends while A's Postgres is away, so its record waits in A's spool
    // and A dies before it can ship it.
    let (_call, _answered) = connected(&alice, &bob, &cast.bob.extension).await;
    // A keeps the record of a call that is up every ten seconds.
    let open = dir.join("e2e-fault-a-cdr-spool").join("open");
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::fs::read_dir(&open).map(|d| d.count()).unwrap_or(0) == 0 {
        assert!(
            Instant::now() < deadline,
            "A keeps no open record of Alice's call"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let (short, _) = connected(&dave, &alice, &cast.alice.extension).await;
    a_db.cut();
    tokio::time::sleep(Duration::from_secs(1)).await;
    hung_up(&short, &alice, "on A with its Postgres away").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let a_logs = a.kill().await;
    let killed = Instant::now();
    a_db.restore().await;

    // B puts Carol through to Bob.
    let deadline = killed + Duration::from_secs(LEASE_TTL_SECS + 5);
    let through = loop {
        match try_call(&carol, &bob, &cast.bob.extension, SOON).await {
            Ok(c) => {
                c.hangup().await.unwrap();
                break Ok(killed.elapsed());
            }
            Err(code) if Instant::now() > deadline => break Err(code),
            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    };
    if let Err(code) = through {
        let logs = b.logs().await;
        panic!(
            "Bob is still busy {:?} after the node carrying his call died \
             (last answer {code:?})\n{logs}\n{a_logs}",
            killed.elapsed()
        );
    }

    // A has left the cluster.
    let deadline = killed + Duration::from_secs(6);
    while fcp_testkit::cluster::heartbeat(&redis, "e2e-fault-a")
        .await
        .is_some()
    {
        assert!(
            Instant::now() < deadline,
            "A is still a member {:?} after it died",
            killed.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // A again, on its spool: the record it could not write is written,
    // and so is the call it was carrying when it died, answered and
    // ended by the node's failure.
    let before = cast.seed.cdrs(Some(CdrKind::Call), 0, SOON).await.len();
    a.restart("e2e-fault-a", &a_url, Some(&redis), dir).await;
    let deadline = Instant::now() + CAUGHT_UP;
    loop {
        let records = cast.seed.cdrs(Some(CdrKind::Call), 0, SOON).await;
        let dave_alice = records
            .iter()
            .any(|r| r.from_uri.contains("dave") && r.to_uri.contains("1001"));
        let alice_bob = records
            .iter()
            .find(|r| r.from_uri.contains("alice") && r.to_uri.contains("1002"));
        if let (true, Some(alice_bob)) = (dave_alice, alice_bob) {
            assert_eq!(
                alice_bob.disposition,
                fcp_core::storage::CallDisposition::Answered,
                "{alice_bob:#?}"
            );
            assert!(
                alice_bob.call_duration_ms.unwrap_or(0) > 0,
                "{alice_bob:#?}"
            );
            assert!(
                alice_bob
                    .sip_reason
                    .as_deref()
                    .is_some_and(|r| r.contains("node stopped")),
                "{alice_bob:#?}"
            );
            break;
        }
        if Instant::now() > deadline {
            let logs = a.logs().await;
            panic!(
                "the record A spooled before it died is not written \
                 ({before} records before its restart): {records:#?}\n{logs}"
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    a.finish().await;
    b.finish().await;
    cast.finish().await;
}

/// A Postgres pair, or skip.
macro_rules! pair_or_skip {
    () => {
        match PgPair::start().await {
            Some(p) => p,
            None => {
                eprintln!("skipped: the Postgres server's binaries are not installed");
                return;
            }
        }
    };
}

/// Carol registers again, within [`CAUGHT_UP`].
async fn registers_again(carol: &Phone, node: &mut Node, pair: &PgPair, when: &str) {
    let deadline = Instant::now() + CAUGHT_UP;
    loop {
        match register_timed(carol).await {
            (Some(200), _) => return,
            _ if Instant::now() > deadline => {
                let logs = node.logs().await;
                panic!("Carol cannot register {when}\n{logs}\n{}", pair.logs());
            }
            _ => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// The primary dies while Alice and Bob talk, and its standby is promoted
/// in its place (D5c). The node is given the primary by an address the
/// failover moves (a VIP, a DNS name, a pooler: here a [`CutLink`]), and
/// reads its configuration from the standby, as a region does.
///
/// - The call lives, and its BYE crosses once the new primary is up.
/// - While there is no primary, a REGISTER is answered 503 within
///   [`ANSWERED_WITHIN`].
/// - Once the standby is promoted and the address moved, Carol registers
///   and calls Dave, a call to Alice finds the registration made before
///   the failover, and the call's record is written, with no restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_primary_fails_over_to_its_standby_mid_call() {
    let pair = pair_or_skip!();
    // The address the node and the test write through moves with the
    // failover.
    let (link, primary) = CutLink::for_url(&pair.primary_url()).await;
    let cast = Cast::on(&primary).await;
    let mut node = Node::start_with(
        "e2e-failover",
        &primary,
        None,
        cast.dir.path(),
        &format!("read_replica_url = \"{}\"", pair.standby_url()),
    )
    .await;
    let alice = cast.phone(&cast.alice, &node).await;
    let bob = cast.phone(&cast.bob, &node).await;
    let carol = cast.phone(&cast.carol, &node).await;
    let dave = cast.phone(&cast.dave, &node).await;
    let (call, answered) = connected(&alice, &bob, &cast.bob.extension).await;

    // Everything written so far is on the standby; the primary dies.
    pair.caught_up(SOON).await;
    pair.crash_primary().await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    both_ways(&call, &answered, "with no primary").await;
    let (code, took) = register_timed(&carol).await;
    let (ready_away, ready_body) = node.ready().await;

    // The standby is promoted and the address moves to it.
    pair.promote_standby().await;
    link.retarget(pair.standby_addr());
    both_ways(&call, &answered, "after the failover").await;
    hung_up(&call, &bob, "after the failover").await;

    registers_again(&carol, &mut node, &pair, "after the failover").await;
    match try_call(&carol, &dave, &cast.dave.extension, SOON).await {
        Ok(call) => {
            call.hangup().await.unwrap();
        }
        Err(code) => {
            let logs = node.logs().await;
            panic!("Carol cannot call Dave after the failover: {code:?}\n{logs}");
        }
    }
    // Alice registered before the failover; the new primary has it.
    match try_call(&dave, &alice, &cast.alice.extension, SOON).await {
        Ok(call) => {
            call.hangup().await.unwrap();
        }
        Err(code) => {
            let logs = node.logs().await;
            panic!("Alice is not reachable after the failover: {code:?}\n{logs}");
        }
    }
    let recorded = cast.calls_recorded(3, CAUGHT_UP).await;

    if took > ANSWERED_WITHIN || code != Some(503) || ready_away != 503 {
        let logs = node.logs().await;
        panic!(
            "with no primary, a REGISTER was answered {code:?} in {took:?}; \
             /ready {ready_away} {ready_body}\n{logs}"
        );
    }
    assert!(recorded >= 3, "{recorded} calls recorded");
    let (ready_back, body) = node.ready().await;
    assert_eq!(ready_back, 200, "ready after the failover: {body}");
    node.finish().await;
}

/// The node's primary address leads to a standby: one the failover has
/// not promoted yet, or the old primary come back as a standby while a
/// node still resolves the old address (D5c). The standby answers every
/// query and refuses every write.
///
/// - A REGISTER is answered 503 within [`ANSWERED_WITHIN`], as with no
///   primary, and `/ready` says the database is not writable.
/// - A call is still put through: it only reads, and its record waits
///   in the spool for a primary.
/// - Once the standby is promoted, Carol registers with no restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_standby_behind_the_primary_address() {
    let pair = pair_or_skip!();
    let (link, primary) = CutLink::for_url(&pair.primary_url()).await;
    let cast = Cast::on(&primary).await;
    let mut node = Node::start("e2e-standby", &primary, None, cast.dir.path()).await;
    let carol = cast.phone(&cast.carol, &node).await;
    let dave = cast.phone(&cast.dave, &node).await;
    pair.caught_up(SOON).await;

    // The address moves to the standby before it is promoted; the node's
    // connections to the primary are dropped, as a moved VIP drops them.
    link.retarget(pair.standby_addr());
    link.cut();
    link.restore().await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (code, took) = register_timed(&carol).await;
    let away_call = match try_call(&carol, &dave, &cast.dave.extension, ANSWERED_WITHIN).await {
        Ok(c) => {
            let _ = c.hangup().await;
            "answered".to_string()
        }
        Err(code) => format!("{code:?}"),
    };
    let (ready_away, ready_body) = node.ready().await;

    pair.promote_standby().await;
    registers_again(&carol, &mut node, &pair, "once the standby is promoted").await;
    let recorded = cast.calls_recorded(1, CAUGHT_UP).await;

    if took > ANSWERED_WITHIN || code != Some(503) || ready_away != 503 || away_call != "answered" {
        let logs = node.logs().await;
        panic!(
            "with a standby at the primary's address, a REGISTER was answered \
             {code:?} in {took:?}; Carol to Dave: {away_call}; \
             /ready {ready_away} {ready_body}\n{logs}"
        );
    }
    assert_eq!(ready_body["database"]["writable"], false, "{ready_body}");
    assert!(recorded >= 1, "the call's record is written once promoted");
    node.finish().await;
}
