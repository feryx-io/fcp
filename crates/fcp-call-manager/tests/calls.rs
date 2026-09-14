//! Calls through the call manager over real SIP and RTP
//! (`docs/TEST_FLOOR.md`, T1): phones register, call each other, hear
//! each other and hang up, against the binary on loopback with a
//! throwaway database. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);

/// A running call manager on a fresh database, with what a test needs
/// to reach it.
struct Stack {
    service: Option<Service>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database to test against.
    async fn start() -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "e2e-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 5

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            service: Some(service),
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

    async fn get(&self, path: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(code, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
    }

    /// Give up: print the service's logs and what `phone` saw, then panic.
    async fn fail(mut self, why: &str, phone: &Phone) -> ! {
        let mut seen = Vec::new();
        while let Some(ev) = phone.next_event(Duration::from_millis(50)).await {
            seen.push(ev);
        }
        let logs = match self.service.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
        panic!("{why}\nphone {} saw: {seen:#?}\n{logs}", phone.username);
    }

    /// Stop the service (a clean exit is asserted) and drop the database.
    async fn finish(mut self) {
        if let Some(service) = self.service.take() {
            service.stop().await;
        }
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

macro_rules! stack_or_skip {
    () => {
        match Stack::start().await {
            Some(s) => s,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registration_is_digest_authenticated_and_listed() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;

    // The right password registers; the registration is listed.
    let phone = stack.phone(&alice).await;
    assert_eq!(phone.register().await.unwrap(), 200);
    let regs = stack.get("/registrations").await;
    assert_eq!(regs["count"], 1, "{regs}");
    assert!(
        regs["registrations"][0]["aor"]
            .as_str()
            .unwrap_or("")
            .contains("alice@"),
        "{regs}"
    );

    // The wrong password does not, nor does a user that does not exist.
    let wrong = Phone::new("alice", "not-her-password", REALM, stack.sip)
        .await
        .unwrap();
    let code = wrong.register().await.unwrap();
    assert!(matches!(code, 401 | 403), "wrong password answered {code}");
    let nobody = Phone::new("nobody", "x", REALM, stack.sip).await.unwrap();
    let code = nobody.register().await.unwrap();
    assert!(!(200..300).contains(&code), "unknown user answered {code}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_is_answered_heard_both_ways_and_recorded() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    // Alice dials Bob's extension; Bob rings, then answers.
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let why = format!(
            "Bob's phone did not ring; Alice's call is {:?}",
            alice_call.phase()
        );
        stack.fail(&why, &alice_phone).await;
    };
    assert!(incoming.from.contains("alice"), "{}", incoming.from);
    assert!(
        alice_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Ringing { code: 180, .. }))
            .await
            .is_some(),
        "Alice hears ringing"
    );
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Audio both ways: a tone from each side heard at the other.
    alice_call.media().play_tone(440.0, 1500);
    assert!(
        bob_call.media().heard(440, SOON).await,
        "Bob hears Alice's 440 Hz (received {} packets)",
        bob_call.media().packets_received()
    );
    bob_call.media().play_tone(880.0, 1500);
    assert!(
        alice_call.media().heard(880, SOON).await,
        "Alice hears Bob's 880 Hz (received {} packets)",
        alice_call.media().packets_received()
    );

    // The call is visible while it is up.
    let calls = stack.get("/calls").await;
    assert!(
        calls["calls"]
            .as_array()
            .map(|c| !c.is_empty())
            .unwrap_or(false)
            || calls["count"].as_u64().unwrap_or(0) > 0,
        "{calls}"
    );

    // Alice hangs up; Bob's phone gets the BYE.
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Bob's phone gets BYE"
    );
    assert!(bob_call.ended(SOON).await);

    // And a CDR says the call was answered.
    let deadline = tokio::time::Instant::now() + SOON;
    let cdr = loop {
        let cdrs = stack.get("/cdrs").await;
        if let Some(first) = cdrs["cdrs"].as_array().and_then(|c| c.first()) {
            break first.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "no CDR: {cdrs}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert!(cdr["call_duration_ms"].as_u64().unwrap_or(0) > 0, "{cdr}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_callee_can_hang_up() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let bob_call = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .answer()
        .await
        .unwrap();
    alice_call.answered(SOON).await.unwrap();

    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    assert!(
        alice_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Alice's phone gets BYE"
    );
    assert!(alice_call.ended(SOON).await);

    stack.finish().await;
}
