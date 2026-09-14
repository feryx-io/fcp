//! Calls through the call manager over real SIP and RTP
//! (`docs/TEST_FLOOR.md`, T1 and T2): phones register, call each other,
//! hear each other, hold, send digits, transfer and hang up, against the
//! binary on loopback with a throwaway database. Skipped without
//! `FCP_TEST_DATABASE_URL`.

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
    service: tokio::sync::Mutex<Option<Service>>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database to test against.
    async fn start() -> Option<Self> {
        Self::start_with("", 40).await
    }

    /// As [`Stack::start`], with `extra` TOML appended to the config and
    /// `rtp_ports` in the media range (a queue wants at least 100).
    async fn start_with(extra: &str, rtp_ports: u16) -> Option<Self> {
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

[logging]
level = "info"
format = "compact"

{extra}
"#,
            db = db.url(),
            rtp_end = rtp_start + rtp_ports,
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

    /// The records at `/cdrs`, once at least `n` are there.
    async fn cdrs(&self, n: usize, within: Duration) -> Vec<serde_json::Value> {
        self.cdrs_at("/cdrs", n, within).await
    }

    /// The records at `path` (`/cdrs?…`), once at least `n` are there.
    async fn cdrs_at(&self, path: &str, n: usize, within: Duration) -> Vec<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let body = self.get(path).await;
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

    async fn get(&self, path: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(code, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
    }

    /// Give up: print the service's logs and what `phone` saw, then panic.
    async fn fail(&self, why: &str, phone: &Phone) -> ! {
        let mut seen = Vec::new();
        while let Some(ev) = phone.next_event(Duration::from_millis(50)).await {
            seen.push(ev);
        }
        let logs = match self.service.lock().await.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        panic!("{why}\nphone {} saw: {seen:#?}\n{logs}", phone.username);
    }

    /// Stop the service (a clean exit is asserted) and drop the database.
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

    // And the call's record says what happened (docs/CDR_PIPELINE.md §5):
    // answered, hung up by the caller, the codec, media both ways, the node.
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["kind"], "call", "{cdr}");
    assert!(
        cdr["cdr_id"].as_str().is_some_and(|id| !id.is_empty()),
        "{cdr}"
    );
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert_eq!(cdr["hangup_cause"], "NormalClearing", "{cdr}");
    assert_eq!(cdr["hangup_party"], "Caller", "{cdr}");
    assert!(cdr["call_duration_ms"].as_u64().unwrap_or(0) > 0, "{cdr}");
    assert_eq!(cdr["codec_a_leg"], "PCMU", "{cdr}");
    assert_eq!(cdr["node_id"], "e2e-call-manager", "{cdr}");
    assert!(
        cdr["rtp_packets_received"].as_u64().unwrap_or(0) > 0
            && cdr["rtp_packets_sent"].as_u64().unwrap_or(0) > 0,
        "the anchored media's counters are on the record: {cdr}"
    );
    // The pipeline's status: the spool drained (the cursor commits just
    // after the record lands), one record stored here.
    let deadline = tokio::time::Instant::now() + SOON;
    let status = loop {
        let status = stack.get("/cdr/status").await;
        if status["spool"]["pending_bytes"] == 0 {
            break status;
        }
        assert!(tokio::time::Instant::now() < deadline, "{status}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(status["processor"]["stored"], 1, "{status}");

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

    // The record knows it was the callee who hung up.
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert_eq!(cdr["hangup_party"], "Callee", "{cdr}");

    stack.finish().await;
}

/// Two registered phones with the callee answered: the start of most
/// feature scenarios.
async fn answered_call(stack: &Stack) -> (Phone, fcp_testkit::Call, Phone, fcp_testkit::Call) {
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let why = format!(
            "Bob's phone did not ring; Alice's call is {:?}",
            alice_call.phase()
        );
        stack.fail(&why, &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    if let Err(e) = alice_call.answered(SOON).await {
        let why = format!("Alice's call was not answered: {e}");
        stack.fail(&why, &alice_phone).await;
    }
    (alice_phone, alice_call, bob_phone, bob_call)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_and_decline_reach_the_caller() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    for (code, reason) in [(486, "Busy Here"), (603, "Decline")] {
        let call = alice_phone.call(&bob.extension).await.unwrap();
        let incoming = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
        incoming.reject(code, reason).await.unwrap();
        match call.answered(SOON).await {
            Err(err) if err.to_string().contains(&code.to_string()) => {}
            other => {
                let why = format!("{code} {reason} did not reach the caller: {other:?}");
                stack.fail(&why, &alice_phone).await;
            }
        }
    }

    // Each rejection is on the record as what the callee sent, not as a
    // guess from the failure text.
    let cdrs = stack.cdrs(2, SOON).await;
    let by_status = |code: u64| {
        cdrs.iter()
            .find(|c| c["sip_status_code"].as_u64() == Some(code))
            .unwrap_or_else(|| panic!("no record with status {code}: {cdrs:#?}"))
    };
    let busy = by_status(486);
    assert_eq!(busy["disposition"], "Busy", "{busy}");
    assert_eq!(busy["hangup_cause"], "UserBusy", "{busy}");
    assert_eq!(busy["hangup_party"], "Callee", "{busy}");
    let declined = by_status(603);
    assert_eq!(declined["disposition"], "Failed", "{declined}");
    assert_eq!(declined["hangup_cause"], "CallRejected", "{declined}");
    assert_eq!(declined["hangup_party"], "Callee", "{declined}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unanswered_call_times_out_and_the_callee_is_cancelled() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let call = alice_phone.call(&bob.extension).await.unwrap();
    let ringing = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .leave_ringing();
    // [b2bua.forking] timeout = 3: the caller hears the timeout (the
    // B2BUA answers 480 for a callee who did not pick up), the callee's
    // ringing INVITE is cancelled.
    match call.answered(Duration::from_secs(10)).await {
        Err(err) if err.to_string().contains("480") || err.to_string().contains("408") => {}
        other => {
            let why = format!("no ring timeout reached the caller: {other:?}");
            stack.fail(&why, &alice_phone).await;
        }
    }
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Cancelled { .. }))
            .await
            .is_some(),
        "Bob's phone was cancelled"
    );
    assert!(ringing.ended(SOON).await);

    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["disposition"], "NoAnswer", "{cdr}");
    assert_eq!(cdr["hangup_cause"], "NoAnswer", "{cdr}");
    assert!(cdr["answer_time_epoch"].is_null(), "{cdr}");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_record_survives_the_database_going_away_under_the_call() {
    let stack = stack_or_skip!();
    let (_alice_phone, alice_call, bob_phone, _bob_call) = answered_call(&stack).await;

    // The CDR table goes away while the call is up: the database refuses
    // the record when the call ends (docs/CDR_PIPELINE.md §6).
    stack
        .seed
        .execute("ALTER TABLE cdrs RENAME TO cdrs_away")
        .await;
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Bob's phone gets BYE"
    );
    // Long enough for the shipper to have tried and backed off.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // The table comes back; the spooled record lands, once.
    stack
        .seed
        .execute("ALTER TABLE cdrs_away RENAME TO cdrs")
        .await;
    let cdrs = stack.cdrs(1, Duration::from_secs(20)).await;
    assert_eq!(cdrs.len(), 1, "{cdrs:#?}");
    assert_eq!(cdrs[0]["disposition"], "Answered", "{}", cdrs[0]);
    assert_eq!(cdrs[0]["hangup_party"], "Caller", "{}", cdrs[0]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(stack.cdrs(1, SOON).await.len(), 1, "no second copy");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_to_an_unregistered_user_fails() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    let call = alice_phone.call(&carol.extension).await.unwrap();
    let err = call.answered(SOON).await.expect_err("nobody registered");
    assert!(
        err.to_string().contains("480") || err.to_string().contains("404"),
        "{err}"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hold_and_resume_are_relayed_to_the_far_end() {
    let stack = stack_or_skip!();
    let (_alice_phone, alice_call, bob_phone, _bob_call) = answered_call(&stack).await;

    assert_eq!(alice_call.hold().await.unwrap(), 200);
    let held = bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .expect("Bob's phone gets the hold re-INVITE");
    let PhoneEvent::ReInvite { sdp, .. } = held else {
        unreachable!()
    };
    assert!(sdp.contains("a=sendonly"), "hold SDP relayed: {sdp}");

    assert_eq!(alice_call.resume().await.unwrap(), 200);
    let resumed = bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .expect("Bob's phone gets the resume re-INVITE");
    let PhoneEvent::ReInvite { sdp, .. } = resumed else {
        unreachable!()
    };
    assert!(sdp.contains("a=sendrecv"), "resume SDP relayed: {sdp}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtmf_crosses_as_rfc_2833_and_as_info() {
    let stack = stack_or_skip!();
    let (_alice_phone, alice_call, bob_phone, bob_call) = answered_call(&stack).await;

    // RFC 2833 in the media.
    alice_call.media().send_dtmf('5');
    if bob_call.media().heard_dtmf(SOON).await != Some('5') {
        let why = format!(
            "Bob did not hear the digit (Alice sent {} packets, Bob received {})",
            alice_call.media().packets_sent(),
            bob_call.media().packets_received()
        );
        stack.fail(&why, &_alice_phone).await;
    }

    // SIP INFO dtmf-relay, relayed to the far phone.
    assert_eq!(alice_call.send_info_dtmf('7').await.unwrap(), 200);
    let info = bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::Info { .. }))
        .await
        .expect("Bob's phone gets the INFO");
    let PhoneEvent::Info { body, .. } = info else {
        unreachable!()
    };
    assert!(body.contains("Signal=7"), "{body}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blind_transfer_moves_the_caller_to_a_third_phone() {
    let stack = stack_or_skip!();
    let (alice_phone, alice_call, bob_phone, bob_call) = answered_call(&stack).await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let carol_phone = stack.phone(&carol).await;
    assert_eq!(carol_phone.register().await.unwrap(), 200);
    alice_phone.follow_refers(true);

    // Bob transfers Alice to Carol: the REFER reaches Alice's phone,
    // which dials Carol.
    let refer_to = format!("sip:{}@{}", carol.extension, stack.sip);
    assert_eq!(bob_call.refer(&refer_to).await.unwrap(), 202);
    assert!(
        alice_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Refer { .. }))
            .await
            .is_some(),
        "Alice's phone gets the REFER"
    );
    let carol_call = carol_phone
        .incoming(Duration::from_secs(10))
        .await
        .expect("Carol's phone rings")
        .answer()
        .await
        .unwrap();

    // Alice and Carol hear each other; Bob's leg is dropped.
    carol_call.media().play_tone(660.0, 1500);
    let alice_new = alice_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::Answered { .. }))
        .await
        .expect("Alice's new call is answered");
    let _ = alice_new;
    assert!(
        bob_phone
            .expect(Duration::from_secs(10), |e| matches!(
                e,
                PhoneEvent::Bye { .. }
            ))
            .await
            .is_some(),
        "Bob's phone gets BYE once the transfer succeeds"
    );
    assert!(alice_call.ended(SOON).await, "Alice's old leg is over");

    assert_eq!(carol_call.hangup().await.unwrap(), 200);

    // Two call records, and the new call knows the call it came from
    // (docs/CDR_PIPELINE.md §5): the Refer-To this call manager relayed
    // carried it, and Alice's phone copied it into the new INVITE.
    let records = stack.cdrs(2, SOON).await;
    let with_bob = records
        .iter()
        .find(|r| r["to_uri"].as_str().unwrap_or("").contains(":1002@"))
        .unwrap_or_else(|| panic!("no record of the call to Bob: {records:#?}"));
    let with_carol = records
        .iter()
        .find(|r| r["to_uri"].as_str().unwrap_or("").contains(":1003@"))
        .unwrap_or_else(|| panic!("no record of the call to Carol: {records:#?}"));
    assert_eq!(
        with_carol["correlation_id"], with_bob["call_id"],
        "the transferred call points at the call it left: {with_carol}"
    );
    assert!(with_bob["correlation_id"].is_null(), "{with_bob}");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queued_caller_leaves_a_call_record_and_a_queue_record_under_one_id() {
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true

[queue.queues.support]
name = "support"
extension = "7000"
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    // Alice calls the queue; it answers her and nobody takes the call.
    let call = alice_phone.call("7000").await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("the queue did not answer: {e}"), &alice_phone)
            .await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);

    // The call and its time in the queue, recorded under one call id; the
    // default listing shows the call alone.
    let records = stack.cdrs_at("/cdrs?kind=all", 2, SOON).await;
    assert_eq!(records.len(), 2, "{records:#?}");
    let call_record = records
        .iter()
        .find(|r| r["kind"] == "call")
        .expect("call record");
    let queue_record = records
        .iter()
        .find(|r| r["kind"] == "queue")
        .expect("queue record");
    assert_eq!(
        call_record["call_id"], queue_record["call_id"],
        "{records:#?}"
    );
    assert_eq!(call_record["disposition"], "Answered", "{call_record}");
    assert_eq!(queue_record["disposition"], "Cancelled", "{queue_record}");
    let calls_only = stack.cdrs(1, SOON).await;
    assert_eq!(calls_only.len(), 1, "{calls_only:#?}");
    assert_eq!(calls_only[0]["kind"], "call");

    // And the call is gone from the live calls.
    let live = stack.get("/calls").await;
    assert_eq!(live["count"].as_u64().unwrap_or(0), 0, "{live}");
    stack.finish().await;
}
