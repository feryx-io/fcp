//! Enterprise calling, phase 1 (`docs/ENTERPRISE_CALLING.md` §3.1): a
//! call to a user follows the user's calling settings whichever path it
//! takes — forward all, busy, no answer and unreachable, do not disturb, a
//! user with no phone reaching voicemail, a flow dialing a user, and the
//! feature codes that change the settings from a phone — against the
//! binary over real SIP. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{LineBindingStore, PhoneNumberStore, VoicemailStore};
use fcp_core::{CallTarget, CallingSettings, CallingStore};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// Long enough for a ring time to run out and the forward to land.
const RING_OUT: Duration = Duration::from_secs(15);
/// Long enough to be sure a phone that should not ring did not.
const QUIET: Duration = Duration::from_secs(2);
/// A note of the built-in hold music (its E, 659.25 Hz): the testkit's
/// detector works in 50 Hz bins, where the C at 523.25 Hz leaks away.
const DEFAULT_MUSIC_HZ: u32 = 659;

struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    http: u16,
    /// Where the voicemail server would be: a testkit phone answers here.
    voicemail: SocketAddr,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database to test against.
    async fn start() -> Option<Self> {
        Self::start_with("").await
    }

    /// As [`Stack::start`], with `extra` TOML appended to the config.
    async fn start_with(extra: &str) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let voicemail_port = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "e2e-calling"

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

[b2bua]
voicemail_server_uri = "sip:voicemail@127.0.0.1:{voicemail_port}"

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

{extra}
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
            voicemail: SocketAddr::from(([127, 0, 0, 1], voicemail_port)),
            _dir: dir,
        })
    }

    async fn phone(&self, user: &SeededUser) -> Phone {
        let phone = Phone::new(&user.username, &user.password, REALM, self.sip)
            .await
            .expect("phone binds");
        assert_eq!(phone.register().await.unwrap(), 200);
        phone
    }

    /// The voicemail server: a phone at the address the node forwards
    /// voicemail to, which never registers.
    async fn voicemail_server(&self) -> Phone {
        Phone::bound_to(
            &self.voicemail.to_string(),
            "voicemail",
            "voicemail",
            REALM,
            self.sip,
        )
        .await
        .expect("voicemail stub binds")
    }

    /// Set a user's calling settings as the API would.
    async fn calling(&self, user: &SeededUser, set: impl FnOnce(&mut CallingSettings)) {
        let storage = self.seed.storage();
        let row = fcp_core::storage::CredentialStore::get_user_in_tenant(
            storage,
            &user.username,
            &self.seed.tenant_id,
        )
        .await
        .expect("read user")
        .expect("seeded user");
        let mut settings = CallingSettings::defaults(self.seed.tenant_id, row.id);
        set(&mut settings);
        settings.validate().expect("valid settings");
        storage
            .put_calling_settings(settings)
            .await
            .expect("store settings");
    }

    /// Give a user a number, a line binding and an enabled mailbox.
    async fn mailbox(&self, user: &SeededUser, e164: &str) {
        let storage = self.seed.storage();
        let row = fcp_core::storage::CredentialStore::get_user_in_tenant(
            storage,
            &user.username,
            &self.seed.tenant_id,
        )
        .await
        .expect("read user")
        .expect("seeded user");
        let number =
            fcp_core::PhoneNumber::new(e164.to_string(), self.seed.tenant_id).expect("number");
        let binding = fcp_core::LineBinding::new(
            number.id,
            Some(row.id),
            fcp_core::BindingType::User,
            self.seed.tenant_id,
        );
        let mailbox = fcp_core::VoicemailMailbox::new(
            self.seed.tenant_id,
            row.id,
            binding.id,
            user.extension.clone(),
            "1234".to_string(),
        );
        storage.put_phone_number(number).await.expect("number");
        storage.put_line_binding(binding).await.expect("binding");
        storage.put_mailbox(mailbox).await.expect("mailbox");
    }

    /// The node's view of its calls (`GET /calls`).
    async fn calls(&self) -> Vec<serde_json::Value> {
        let (code, body) = fcp_testkit::smoke::http_get(&loopback(self.http, "/calls"))
            .await
            .expect("GET /calls");
        assert_eq!(code, 200, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("calls JSON");
        json["calls"].as_array().cloned().unwrap_or_default()
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

/// Alice, Bob and Carol with registered phones.
async fn three_phones(stack: &Stack) -> [(SeededUser, Phone); 3] {
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let a = stack.phone(&alice).await;
    let b = stack.phone(&bob).await;
    let c = stack.phone(&carol).await;
    [(alice, a), (bob, b), (carol, c)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forward_all_sends_the_call_on_without_ringing() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    stack
        .calling(&bob, |s| {
            s.forward_all = Some(CallTarget::Extension(carol.extension.clone()))
        })
        .await;

    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring for Bob's forward", &alice)
            .await;
    };
    assert!(
        bob_phone
            .incoming(Duration::from_millis(300))
            .await
            .is_none(),
        "Bob's phone rang although every call is forwarded"
    );
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forward_on_no_answer_after_the_ring_time() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    stack
        .calling(&bob, |s| {
            s.forward_no_answer = Some(CallTarget::Extension(carol.extension.clone()));
            s.no_answer_secs = Some(5);
        })
        .await;

    let started = Instant::now();
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice).await;
    };
    let ringing = at_bob.leave_ringing();
    let Some(at_carol) = carol_phone.incoming(RING_OUT).await else {
        stack
            .fail("Carol's phone did not ring after Bob's ring time", &alice)
            .await;
    };
    let rang_for = started.elapsed();
    assert!(
        rang_for >= Duration::from_secs(4) && rang_for < Duration::from_secs(12),
        "Bob rang for {rang_for:?}, the setting says 5s"
    );
    assert!(
        ringing.ended(SOON).await,
        "Bob's ringing leg was not cancelled"
    );
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forward_on_busy_when_the_phone_refuses() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    stack
        .calling(&bob, |s| {
            s.forward_busy = Some(CallTarget::Extension(carol.extension.clone()))
        })
        .await;

    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice).await;
    };
    at_bob.reject(486, "Busy Here").await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring after Bob was busy", &alice)
            .await;
    };
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn do_not_disturb_is_busy_without_ringing() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;

    // Nothing to fall back to: the caller hears busy.
    stack.calling(&bob, |s| s.dnd = true).await;
    let call = alice.call(&bob.extension).await.unwrap();
    match call.failed(SOON).await {
        Some(486) => {}
        other => {
            stack
                .fail(&format!("DND did not answer busy: {other:?}"), &alice)
                .await
        }
    }
    assert!(
        bob_phone.incoming(QUIET).await.is_none(),
        "Bob's phone rang in do not disturb"
    );

    // With a busy forward, the forward.
    stack
        .calling(&bob, |s| {
            s.dnd = true;
            s.forward_busy = Some(CallTarget::Extension(carol.extension.clone()));
        })
        .await;
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring for Bob in DND", &alice)
            .await;
    };
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_with_no_phone_reaches_voicemail_by_extension() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    stack.mailbox(&carol, "+15125550103").await;
    let alice_phone = stack.phone(&alice).await;
    let voicemail = stack.voicemail_server().await;

    // Nobody registered for Carol: the call is hers to leave a message on.
    let call = alice_phone.call(&carol.extension).await.unwrap();
    let Some(at_voicemail) = voicemail.incoming(SOON).await else {
        stack
            .fail("the voicemail server was not reached", &alice_phone)
            .await;
    };
    let uri = at_voicemail.request_uri();
    assert!(uri.contains(&carol.extension), "the mailbox dialed: {uri}");
    assert!(uri.contains("reason=unreachable"), "the reason: {uri}");
    let answered = at_voicemail.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches voicemail");
    answered.hangup().await.unwrap();

    // A forward when unreachable wins over the mailbox.
    stack
        .calling(&carol, |s| {
            s.forward_unreachable = Some(CallTarget::Extension(alice.extension.clone()))
        })
        .await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_phone = stack.phone(&bob).await;
    let call = bob_phone.call(&carol.extension).await.unwrap();
    let Some(at_alice) = alice_phone.incoming(SOON).await else {
        stack
            .fail(
                "Alice's phone did not ring for Carol's unreachable forward",
                &bob_phone,
            )
            .await;
    };
    let answered = at_alice.answer().await.unwrap();
    call.answered(SOON).await.expect("Bob reaches Alice");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flow_dials_a_user_and_the_user_answers() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "ring-bob",
        "name": "Ring Bob",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": stack.seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "dial"},
            "dial": {
                "id": "dial", "name": "Dial Bob", "type": "dial",
                "destination": bob.extension, "timeout": 10, "next": "done",
                "on_error": "done", "on_no_answer": "done", "on_busy": "done"
            },
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    let number = fcp_core::PhoneNumber::new("+15125550700".to_string(), stack.seed.tenant_id)
        .expect("valid number");
    let mut binding = fcp_core::LineBinding::new(
        number.id,
        None,
        fcp_core::BindingType::Ivr,
        stack.seed.tenant_id,
    );
    binding.flow_id = Some("ring-bob".to_string());
    stack
        .seed
        .storage()
        .put_phone_number(number)
        .await
        .expect("number");
    stack
        .seed
        .storage()
        .put_line_binding(binding)
        .await
        .expect("binding");

    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let call = alice_phone.call("+15125550700").await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack
            .fail("the flow's dial did not ring Bob", &alice_phone)
            .await;
    };
    let answered = at_bob.answer().await.unwrap();
    // The caller hangs up; the leg the flow dialed ends with the call.
    call.hangup().await.unwrap();
    if !answered.ended(SOON).await {
        stack
            .fail("Bob's leg outlived the caller", &bob_phone)
            .await;
    }

    stack.calling(&bob, |s| s.dnd = true).await;
    let call = alice_phone.call("+15125550700").await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    assert!(
        bob_phone.incoming(QUIET).await.is_none(),
        "Bob's phone rang from a flow while in DND"
    );
    assert!(call.ended(SOON).await, "the flow did not hang up on busy");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn feature_codes_change_the_callers_own_settings() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;

    // *78: do not disturb on. A call to Bob is busy without ringing.
    let code = bob_phone.call("*78").await.unwrap();
    code.answered(SOON).await.expect("*78 accepted");
    let _ = code.hangup().await;
    let call = alice.call(&bob.extension).await.unwrap();
    match call.failed(SOON).await {
        Some(486) => {}
        other => {
            stack
                .fail(&format!("*78 did not set DND: {other:?}"), &alice)
                .await
        }
    }

    // *79: off again, and *72 + Carol's extension: every call to Carol.
    let code = bob_phone.call("*79").await.unwrap();
    code.answered(SOON).await.expect("*79 accepted");
    let _ = code.hangup().await;
    let code = bob_phone
        .call(&format!("*72{}", carol.extension))
        .await
        .unwrap();
    code.answered(SOON).await.expect("*72 accepted");
    let _ = code.hangup().await;
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("*72 did not forward Bob's calls to Carol", &alice)
            .await;
    };
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();

    // *73: forwarding off; Bob rings again.
    let code = bob_phone.call("*73").await.unwrap();
    code.answered(SOON).await.expect("*73 accepted");
    let _ = code.hangup().await;
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack
            .fail("Bob's phone did not ring after *73", &alice)
            .await;
    };
    let answered = at_bob.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Bob");
    answered.hangup().await.unwrap();

    // A bad target is refused and changes nothing.
    let code = bob_phone.call("*72nobody").await.unwrap();
    match code.failed(SOON).await {
        Some(404) => {}
        other => {
            stack
                .fail(&format!("*72 with a bad target: {other:?}"), &bob_phone)
                .await
        }
    }
    let settings = stack
        .seed
        .storage()
        .get_calling_settings(
            &stack.seed.tenant_id,
            &fcp_core::storage::CredentialStore::get_user_in_tenant(
                stack.seed.storage(),
                &bob.username,
                &stack.seed.tenant_id,
            )
            .await
            .unwrap()
            .unwrap()
            .id,
        )
        .await
        .unwrap()
        .expect("Bob has settings");
    assert!(
        settings.forward_all.is_none() && !settings.dnd,
        "{settings:?}"
    );
    stack.finish().await;
}

// ─── Hold, transfer and pickup (docs/ENTERPRISE_CALLING.md §3.2) ─────────

/// `caller` calls `callee`, who answers: both ends of the call.
async fn answered(
    stack: &Stack,
    caller: &Phone,
    callee: &SeededUser,
    callee_phone: &Phone,
) -> (fcp_testkit::Call, fcp_testkit::Call) {
    let call = caller.call(&callee.extension).await.unwrap();
    let Some(incoming) = callee_phone.incoming(SOON).await else {
        stack.fail("the callee's phone did not ring", caller).await;
    };
    let answered = incoming.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    (call, answered)
}

/// A Replaces value as a Refer-To URI header carries it.
fn escaped(replaces: &str) -> String {
    replaces
        .replace('%', "%25")
        .replace('@', "%40")
        .replace(';', "%3B")
        .replace('=', "%3D")
}

/// Wait until the node says the only call on it is held on `leg`.
async fn held_as(stack: &Stack, leg: Option<&str>) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + SOON;
    loop {
        let calls = stack.calls().await;
        let held: Vec<_> = calls.iter().map(|c| c["held"].as_str()).collect();
        if held.contains(&leg) || Instant::now() > deadline {
            return calls;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_knows_a_hold_and_answers_it_recvonly() {
    let Some(stack) = Stack::start_with("[b2bua.media_anchoring]\nmode = \"Always\"\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let (alice_call, _bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;

    // Alice holds Bob: the answer she gets is the node's, and it answers
    // a hold (it used to be `sendrecv`, from her first INVITE).
    let (code, answer) = alice_call.reinvite_answered("sendonly").await.unwrap();
    assert_eq!(code, 200);
    assert!(
        answer.contains("a=recvonly"),
        "the answer to a hold: {answer}"
    );
    let calls = held_as(&stack, Some("b")).await;
    assert!(
        calls.iter().any(|c| c["held"] == "b"),
        "the node does not know Bob is held: {calls:#?}"
    );

    let (code, answer) = alice_call.reinvite_answered("sendrecv").await.unwrap();
    assert_eq!(code, 200);
    assert!(
        !answer.contains("a=recvonly"),
        "the answer to a resume: {answer}"
    );
    let calls = held_as(&stack, None).await;
    assert!(
        calls.iter().all(|c| c["held"].is_null()),
        "the node still thinks a leg is held: {calls:#?}"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_carries_out_a_blind_transfer() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);

    // Bob transfers Alice to Carol: the node dials Carol as Alice, and
    // Alice's phone never sees the REFER.
    let refer_to = format!("sip:{}@{}", carol.extension, stack.sip);
    assert_eq!(bob_call.refer(&refer_to).await.unwrap(), 202);
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring for the transfer", &bob_phone)
            .await;
    };
    assert!(
        at_carol.from.contains("alice"),
        "Carol sees {}",
        at_carol.from
    );
    let carol_call = at_carol.answer().await.unwrap();

    let notified = bob_phone
        .expect(
            SOON,
            |e| matches!(e, PhoneEvent::Notify { body, .. } if body.contains("200")),
        )
        .await;
    assert!(
        notified.is_some(),
        "Bob was not told the transfer succeeded"
    );
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");
    assert!(
        alice
            .expect(Duration::from_millis(500), |e| matches!(
                e,
                PhoneEvent::Refer { .. }
            ))
            .await
            .is_none(),
        "the REFER was relayed to Alice"
    );

    // Alice, on her own call still, hears Carol.
    carol_call.media().play_tone(660.0, 3000);
    assert!(
        alice_call.media().heard(660, SOON).await,
        "Alice does not hear Carol"
    );
    alice_call.hangup().await.unwrap();
    assert!(
        carol_call.ended(SOON).await,
        "Carol's leg outlived the call"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attended_transfer_joins_the_other_two() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let alice = alice;
    let (alice_call, bob_first) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(bob_first.hold().await.unwrap(), 200);
    let (bob_second, carol_call) = answered(&stack, &bob_phone, &carol, &carol_phone).await;

    // Bob completes the transfer: Alice's call replaces his call to Carol.
    let replaces = bob_second.replaces().await.unwrap();
    let refer_to = format!(
        "sip:{}@{}?Replaces={}",
        carol.extension,
        stack.sip,
        escaped(&replaces)
    );
    assert_eq!(bob_first.refer(&refer_to).await.unwrap(), 202);
    assert!(
        bob_first.ended(RING_OUT).await,
        "Bob's leg to Alice was not dropped"
    );
    assert!(
        bob_second.ended(SOON).await,
        "Bob's leg to Carol was not dropped"
    );

    carol_call.media().play_tone(880.0, 3000);
    alice_call.media().play_tone(440.0, 3000);
    assert!(
        alice_call.media().heard(880, SOON).await,
        "Alice does not hear Carol"
    );
    assert!(
        carol_call.media().heard(440, SOON).await,
        "Carol does not hear Alice"
    );
    alice_call.hangup().await.unwrap();
    assert!(
        carol_call.ended(SOON).await,
        "Carol's leg outlived the call"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_parked_by_transfer_is_retrieved_from_its_orbit() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (_carol, carol_phone)] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);

    // Bob parks Alice: a transfer to the park code.
    assert_eq!(
        bob_call
            .refer(&format!("sip:*70@{}", stack.sip))
            .await
            .unwrap(),
        202
    );
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");
    let deadline = Instant::now() + SOON;
    loop {
        let calls = stack.calls().await;
        if calls.iter().any(|c| c["state"] == "Parked") {
            break;
        }
        if Instant::now() > deadline {
            stack
                .fail(&format!("no parked call: {calls:#?}"), &bob_phone)
                .await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !alice_call.ended(Duration::from_millis(200)).await,
        "Alice was hung up"
    );
    assert!(
        alice_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await,
        "Alice, parked, hears no music"
    );

    // Carol takes her from orbit 701.
    let carol_call = carol_phone.call("*71701").await.unwrap();
    if let Err(e) = carol_call.answered(SOON).await {
        stack
            .fail(&format!("the retrieve was not answered: {e}"), &carol_phone)
            .await;
    }
    carol_call.media().play_tone(660.0, 3000);
    assert!(
        alice_call.media().heard(660, SOON).await,
        "Alice does not hear Carol"
    );
    carol_call.hangup().await.unwrap();
    assert!(
        alice_call.ended(SOON).await,
        "Alice's leg outlived the call"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invite_with_replaces_takes_over_a_call() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (_carol, carol_phone)] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;

    // Carol's phone takes Bob's call from him.
    let replaces = bob_call.replaces().await.unwrap();
    let carol_call = carol_phone
        .call_with_headers(&bob.extension, &[("Replaces", replaces.as_str())])
        .await
        .unwrap();
    if let Err(e) = carol_call.answered(SOON).await {
        stack
            .fail(&format!("the Replaces was not answered: {e}"), &carol_phone)
            .await;
    }
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");
    carol_call.media().play_tone(660.0, 3000);
    assert!(
        alice_call.media().heard(660, SOON).await,
        "Alice does not hear Carol"
    );

    // A dialog nobody holds is refused.
    let stranger = carol_phone
        .call_with_headers(
            &bob.extension,
            &[("Replaces", "nobody@host;to-tag=1;from-tag=2")],
        )
        .await
        .unwrap();
    assert_eq!(stranger.failed(SOON).await, Some(481));
    alice_call.hangup().await.unwrap();
    assert!(
        carol_call.ended(SOON).await,
        "Carol's leg outlived the call"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_parked_call_nobody_takes_rings_its_parker_again() {
    let Some(stack) =
        Stack::start_with("[pbx.parking]\nenabled = true\ntimeout_seconds = 2\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    assert_eq!(
        bob_call
            .refer(&format!("sip:*70@{}", stack.sip))
            .await
            .unwrap(),
        202
    );
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");

    // Nobody retrieves it: after the orbit's time the call rings Bob, as
    // Alice, through his own plan.
    let Some(back) = bob_phone.incoming(RING_OUT).await else {
        stack
            .fail("the parked call did not come back to Bob", &bob_phone)
            .await;
    };
    assert!(back.from.contains("alice"), "Bob sees {}", back.from);
    let bob_again = back.answer().await.unwrap();
    bob_again.media().play_tone(660.0, 3000);
    assert!(
        alice_call.media().heard(660, SOON).await,
        "Alice does not hear Bob"
    );
    bob_again.hangup().await.unwrap();
    assert!(
        alice_call.ended(SOON).await,
        "Alice's leg outlived the call"
    );
    stack.finish().await;
}

// ─── Hold music (docs/ENTERPRISE_CALLING.md §3.3) ────────────────────────

/// A 16-bit 8 kHz mono WAV of a steady tone.
fn tone_wav(path: &std::path::Path, hz: f32, secs: f32) {
    let n = (8000.0 * secs) as usize;
    let mut wav = Vec::new();
    let data = (n * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for i in 0..n {
        let s = (2.0 * std::f32::consts::PI * hz * i as f32 / 8000.0).sin() * 0.4;
        wav.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    std::fs::write(path, wav).expect("write wav");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_party_hears_music_and_not_the_holder() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;

    // Bob holds Alice: the node answers him, and Alice's phone is not told.
    let (code, answer) = bob_call.reinvite_answered("sendonly").await.unwrap();
    assert_eq!(code, 200);
    assert!(
        answer.contains("a=recvonly"),
        "the node's answer to a hold: {answer}"
    );
    assert!(
        alice_phone
            .expect(Duration::from_millis(500), |e| matches!(
                e,
                PhoneEvent::ReInvite { .. }
            ))
            .await
            .is_none(),
        "the hold was passed on to Alice's phone"
    );

    // Alice hears the music, and not what Bob's phone keeps sending.
    bob_call.media().play_tone(1400.0, 20_000);
    let got = alice_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await;
    if !got {
        stack
            .fail("Alice, held, hears no music", &alice_phone)
            .await;
    }
    assert!(
        !alice_call
            .media()
            .heard(1400, Duration::from_millis(1500))
            .await,
        "Alice hears Bob while held"
    );

    // Bob takes her back: the music stops and she hears him again.
    let (code, answer) = bob_call.reinvite_answered("sendrecv").await.unwrap();
    assert_eq!(code, 200);
    assert!(
        !answer.contains("a=recvonly"),
        "the node's answer to a resume: {answer}"
    );
    assert!(
        alice_call.media().heard(1400, SOON).await,
        "Alice does not hear Bob after the resume"
    );
    alice_call.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenants_hold_music_plays_unless_the_holder_chose_silence() {
    use fcp_core::prompt::{Prompt, PromptCategory};
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;

    // The tenant's own hold music: a steady 880 Hz.
    let music = stack._dir.path().join("tenant-music.wav");
    tone_wav(&music, 880.0, 4.0);
    use fcp_core::prompt::PromptStore;
    fcp_storage_postgres::prompt::PostgresPromptStore::new(stack.seed.storage().pool().clone())
        .put_prompt(Prompt::new_file(
            stack.seed.tenant_id,
            "tenant-hold-music",
            PromptCategory::Moh,
            "en",
            music,
        ))
        .await
        .expect("seed prompt");

    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    if !alice_call.media().heard(880, SOON).await {
        stack
            .fail("Alice hears no tenant music", &alice_phone)
            .await;
    }
    assert_eq!(bob_call.resume().await.unwrap(), 200);
    alice_call.hangup().await.unwrap();

    // Bob chooses silence: Alice hears neither the tenant's music nor the
    // default.
    stack
        .calling(&bob, |s| {
            s.hold_music = Some(fcp_core::calling::HOLD_MUSIC_NONE.to_string())
        })
        .await;
    // A fresh phone for Alice: a phone's media remembers what it heard.
    let alice_phone = stack.phone(&alice).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    let music = alice_call
        .media()
        .heard(880, Duration::from_millis(1500))
        .await
        || alice_call
            .media()
            .heard(DEFAULT_MUSIC_HZ, Duration::from_millis(500))
            .await;
    if music {
        stack
            .fail("Alice hears music though Bob chose silence", &alice_phone)
            .await;
    }
    alice_call.hangup().await.unwrap();
    stack.finish().await;
}

// ─── Call waiting (docs/ENTERPRISE_CALLING.md §3.4) ──────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_waiting_decides_whether_a_second_call_rings() {
    let stack = stack_or_skip!();
    let [(alice, alice_phone), (bob, bob_phone), (_carol, carol_phone)] =
        three_phones(&stack).await;

    // On by default: a second call rings Bob while he talks to Alice.
    let (alice_call, _bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    let second = carol_phone.call(&bob.extension).await.unwrap();
    let Some(waiting) = bob_phone.incoming(SOON).await else {
        stack
            .fail("the second call did not ring Bob", &carol_phone)
            .await;
    };
    waiting.reject(486, "Busy Here").await.unwrap();
    assert_eq!(second.failed(SOON).await, Some(486));

    // *57: off. The second call is busy without ringing.
    let code = bob_phone.call("*57").await.unwrap();
    code.answered(SOON).await.expect("*57 accepted");
    let _ = code.hangup().await;
    let second = carol_phone.call(&bob.extension).await.unwrap();
    match second.failed(SOON).await {
        Some(486) => {}
        other => {
            stack
                .fail(
                    &format!("call waiting off did not refuse: {other:?}"),
                    &carol_phone,
                )
                .await
        }
    }
    assert!(
        bob_phone.incoming(QUIET).await.is_none(),
        "Bob's phone rang with call waiting off"
    );

    // Off the call, he is free again.
    alice_call.hangup().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let third = carol_phone.call(&bob.extension).await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack
            .fail("Bob, free again, did not ring", &carol_phone)
            .await;
    };
    let answered_third = at_bob.answer().await.unwrap();
    third.answered(SOON).await.expect("Carol reaches Bob");

    // A call Bob placed counts too: while he talks to Carol he is busy to
    // Alice.
    let from_alice = alice_phone.call(&bob.extension).await.unwrap();
    assert_eq!(from_alice.failed(SOON).await, Some(486));
    answered_third.hangup().await.unwrap();
    let _ = alice;
    stack.finish().await;
}
