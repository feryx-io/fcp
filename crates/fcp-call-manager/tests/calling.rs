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
        Self::start_configured(extra, "").await
    }

    /// As [`Stack::start_with`], with `registrar` lines in `[registrar]`.
    async fn start_configured(extra: &str, registrar: &str) -> Option<Self> {
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
{registrar}

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
    /// A mailbox on a number of the user's own; its id.
    async fn mailbox(&self, user: &SeededUser, e164: &str) -> String {
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
        let id = mailbox.id.to_string();
        storage.put_mailbox(mailbox).await.expect("mailbox");
        id
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

/// A call refused before anything rang lets its media session go: the
/// session and its two ports were held from the INVITE for as long as the
/// node ran (`docs/DEEP_TESTING.md` D8 — a carrier's calls to a user with
/// no phone up, refused 480, each kept a session).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_refused_before_it_rang_leaves_no_media_session() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    // Dave has no phone and no mailbox: unreachable, refused at once.
    let dave = stack.seed.user("dave", "dave-pw", "1004").await;
    let alice_phone = stack.phone(&alice).await;
    for _ in 0..3 {
        let call = alice_phone.call(&dave.extension).await.unwrap();
        assert_eq!(call.failed(SOON).await, Some(480));
    }
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let (code, body) = fcp_testkit::smoke::http_get(&loopback(stack.http, "/media-sessions"))
            .await
            .expect("GET /media-sessions");
        assert_eq!(code, 200, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        let left = json["sessions"].as_array().map(Vec::len).unwrap_or(0);
        if left == 0 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{left} media sessions outlived their refused calls: {body}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    stack.finish().await;
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

/// A decline on one of Bob's phones is Bob's answer (a 6xx, RFC 3261
/// §16.7): his other phone stops ringing and the call goes to his busy
/// treatment at once. It rang on to the ring time, then refused the
/// caller (`docs/DEEP_TESTING.md` D8: Anvil declined, the VVX heard
/// ringing, then a fast busy).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_decline_on_one_phone_stops_the_others_and_is_busy() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let bob_desk = stack.phone(&bob).await;
    stack
        .calling(&bob, |s| {
            s.forward_busy = Some(CallTarget::Extension(carol.extension.clone()));
            s.no_answer_secs = Some(30);
        })
        .await;

    let call = alice.call(&bob.extension).await.unwrap();
    let (Some(at_bob), Some(at_desk)) = (
        bob_phone.incoming(SOON).await,
        bob_desk.incoming(SOON).await,
    ) else {
        stack.fail("both of Bob's phones should ring", &alice).await;
    };
    let ringing = at_desk.leave_ringing();
    at_bob.reject(603, "Decline").await.unwrap();
    assert!(
        ringing.ended(SOON).await,
        "Bob's other phone rang on after the decline"
    );
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring after Bob declined", &alice)
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
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("Alice did not reach Carol: {e}"), &alice)
            .await;
    }
    answered.hangup().await.unwrap();
    stack.finish().await;
}

/// A mailbox that hands its calls to a flow (its `flow_id`): a call the
/// mailbox would have taken runs the flow instead — here, ringing someone
/// else — and the voicemail server is not reached (`docs/DEEP_TESTING.md`
/// D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mailbox_that_hands_its_calls_to_a_flow_runs_it() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    stack.mailbox(&carol, "+15125550113").await;
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "carol-away",
        "name": "Carol is away: ring Bob",
        "version": 1,
        "initial_node": "dial",
        "enabled": true,
        "tenant_id": stack.seed.tenant_id.to_string(),
        "nodes": {
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
    let storage = stack.seed.storage();
    let row = fcp_core::storage::CredentialStore::get_user_in_tenant(
        storage,
        &carol.username,
        &stack.seed.tenant_id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut mailbox = storage.get_mailbox_by_user(&row.id).await.unwrap().unwrap();
    mailbox.flow_id = Some("carol-away".to_string());
    storage.put_mailbox(mailbox).await.unwrap();

    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let voicemail = stack.voicemail_server().await;
    // Carol has no phone up: her mailbox would take it, and hands it on.
    let call = alice_phone.call(&carol.extension).await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack
            .fail("the mailbox's flow did not ring Bob", &alice_phone)
            .await;
    };
    assert!(
        voicemail.incoming(QUIET).await.is_none(),
        "the voicemail server was reached"
    );
    let answered = at_bob.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Bob");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

/// A call's own flow (`*98`, voicemail access) is held for the call, not
/// written: every check of a mailbox left a row behind
/// (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voicemail_access_leaves_no_flow_behind() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.mailbox(&alice, "+15125550114").await;
    let alice_phone = stack.phone(&alice).await;
    let call = alice_phone.call("*98").await.unwrap();
    call.answered(SOON).await.expect("voicemail access answers");
    call.hangup().await.unwrap();
    let flows = fcp_flow::FlowStore::list_flows(stack.seed.storage(), None)
        .await
        .unwrap();
    assert!(
        flows.iter().all(|f| !f.id.starts_with("vm-access-")),
        "a stored voicemail access flow: {:?}",
        flows.iter().map(|f| &f.id).collect::<Vec<_>>()
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_with_no_phone_reaches_voicemail_by_extension() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let mailbox_id = stack.mailbox(&carol, "+15125550103").await;
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
    // By the mailbox's id: the voicemail server routes every mailbox by
    // it, where an extension may be one it does not know (D8, 37–38).
    assert!(
        uri.starts_with(&format!("sip:{mailbox_id}@")),
        "the mailbox dialed: {uri}"
    );
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
    dial_code(&stack, &bob_phone, "*78").await;
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
    dial_code(&stack, &bob_phone, "*79").await;
    dial_code(&stack, &bob_phone, &format!("*72{}", carol.extension)).await;
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
    dial_code(&stack, &bob_phone, "*73").await;
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

/// The caller transfers the callee: the callee's leg is offered to the
/// target with the callee's own answer, which the node keeps on its leg.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_caller_transfers_the_callee() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;

    // Alice sends Bob to Carol.
    let refer_to = format!("sip:{}@{}", carol.extension, stack.sip);
    assert_eq!(alice_call.refer(&refer_to).await.unwrap(), 202);
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring for the transfer", &alice)
            .await;
    };
    // Bob is known by the number Alice dialed.
    assert!(
        at_carol.from.contains(&bob.extension),
        "Carol sees {}",
        at_carol.from
    );
    let carol_call = at_carol.answer().await.unwrap();
    let notified = alice
        .expect(
            SOON,
            |e| matches!(e, PhoneEvent::Notify { body, .. } if body.contains("200")),
        )
        .await;
    assert!(
        notified.is_some(),
        "Alice was not told the transfer succeeded"
    );
    assert!(alice_call.ended(SOON).await, "Alice's leg was not dropped");

    // Bob, on his own call still, hears Carol, and she him.
    carol_call.media().play_tone(660.0, 3000);
    assert!(
        bob_call.media().heard(660, SOON).await,
        "Bob does not hear Carol"
    );
    bob_call.media().play_tone(440.0, 3000);
    assert!(
        carol_call.media().heard(440, SOON).await,
        "Carol does not hear Bob"
    );
    bob_call.hangup().await.unwrap();
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

/// An attended transfer to something the node answers itself — a menu, as
/// a VVX's Transfer button does to an IVR (`docs/DEEP_TESTING.md` D8): the
/// consultation call has no second party, only the flow, and the Replaces
/// was refused 481 ("no party to join"). The transferee takes the
/// consulting phone's place on the flow: Alice keys 5 and the menu rings
/// Carol.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attended_transfer_to_a_menu_hands_the_caller_to_it() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let menu = stack.seed.user("menu", "menu-pw", "1500").await;
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "transfer-menu", "name": "Menu", "version": 1, "initial_node": "answer",
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "menu"},
            "menu": {
                "id": "menu", "name": "Menu", "type": "gather", "num_digits": 1,
                "timeout": 20, "transitions": {"5": "dial"}, "default": "menu",
                "on_timeout": "menu"
            },
            "dial": {
                "id": "dial", "name": "Dial Carol", "type": "dial",
                "destination": carol.extension, "timeout": 10, "next": "done",
                "on_error": "done", "on_no_answer": "done", "on_busy": "done"
            },
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    stack
        .calling(&menu, |s| {
            s.forward_all = Some(CallTarget::Flow("transfer-menu".into()))
        })
        .await;

    let (alice_call, bob_first) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(bob_first.hold().await.unwrap(), 200);
    let bob_second = bob_phone.call(&menu.extension).await.unwrap();
    bob_second
        .answered(SOON)
        .await
        .expect("the menu answers Bob");

    // Bob completes the transfer: Alice replaces him on the menu's call.
    let replaces = bob_second.replaces().await.unwrap();
    let refer_to = format!(
        "sip:{}@{}?Replaces={}",
        menu.extension,
        stack.sip,
        escaped(&replaces)
    );
    assert_eq!(bob_first.refer(&refer_to).await.unwrap(), 202);
    assert!(
        bob_second.ended(SOON).await,
        "Bob's leg to the menu was not dropped"
    );
    assert!(
        bob_first.ended(RING_OUT).await,
        "Bob's leg to Alice was not dropped"
    );

    // Alice is on the menu now: her 5 is heard there, and Carol rings.
    tokio::time::sleep(Duration::from_millis(500)).await;
    alice_call.media().send_dtmf('5');
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Alice's key did not reach the menu", &bob_phone)
            .await;
    };
    let carol_call = at_carol.answer().await.unwrap();
    carol_call.media().play_tone(880.0, 3000);
    assert!(
        alice_call.media().heard(880, SOON).await,
        "Alice does not hear Carol"
    );
    alice_call.hangup().await.unwrap();
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

/// A menu's timeout runs from the end of its prompt: a caller who listens
/// to the whole prompt, then presses, is heard. It ran from the prompt's
/// start, so the D8 menu — a prompt about as long as its 8 s timeout —
/// said goodbye as it finished asking (`docs/DEEP_TESTING.md` D8). A key
/// during the prompt still cuts it short.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_menus_timeout_runs_from_the_end_of_its_prompt() {
    const PROMPT_HZ: u32 = 500;
    let stack = stack_or_skip!();
    let [(_, alice), (carol, carol_phone), _] = three_phones(&stack).await;
    let menu = stack.seed.user("menu", "menu-pw", "1500").await;
    let prompt = stack._dir.path().join("long-prompt.wav");
    tone_wav(&prompt, PROMPT_HZ as f32, 4.0);
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "long-menu", "name": "Menu", "version": 1, "initial_node": "answer",
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "menu"},
            "menu": {
                "id": "menu", "name": "Menu", "type": "gather", "num_digits": 1,
                "timeout": 2, "barge_in": true,
                "prompt": {"type": "play", "url": prompt.display().to_string()},
                "transitions": {"3": "dial"}, "default": "done", "on_timeout": "done"
            },
            "dial": {
                "id": "dial", "name": "Dial Carol", "type": "dial",
                "destination": carol.extension, "timeout": 10, "next": "done",
                "on_error": "done", "on_no_answer": "done", "on_busy": "done"
            },
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    stack
        .calling(&menu, |s| {
            s.forward_all = Some(CallTarget::Flow("long-menu".into()))
        })
        .await;

    // The whole 4 s prompt heard, then 3: past the old deadline (2 s from
    // the prompt's start), inside the new one (2 s from its end).
    let call = alice.call(&menu.extension).await.unwrap();
    call.answered(SOON).await.expect("the menu answers");
    assert!(
        call.media().heard(PROMPT_HZ, SOON).await,
        "the prompt is not heard"
    );
    tokio::time::sleep(Duration::from_millis(4500)).await;
    assert!(
        !call.ended(Duration::ZERO).await,
        "the menu hung up while the prompt was still playing or just after"
    );
    call.media().send_dtmf('3');
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("the 3 pressed after the prompt was not heard", &alice)
            .await;
    };
    at_carol.answer().await.unwrap().hangup().await.unwrap();
    let _ = call.ended(SOON).await;

    // Pressed during the prompt: the prompt stops and the choice is taken.
    let call = alice.call(&menu.extension).await.unwrap();
    call.answered(SOON).await.expect("the menu answers");
    assert!(call.media().heard(PROMPT_HZ, SOON).await);
    call.media().send_dtmf('3');
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("a 3 pressed during the prompt was not heard", &alice)
            .await;
    };
    at_carol.answer().await.unwrap().hangup().await.unwrap();
    stack.finish().await;
}

/// A menu asked up to `max_attempts` times: nothing pressed plays the
/// no-input prompt and the menu again, a key it has no branch for plays the
/// invalid-choice prompt and the menu again, and when the attempts are
/// spent the call goes to `on_max_attempts`. Asked once, it said goodbye
/// at the first silence (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_menu_asks_again_then_gives_up_where_it_is_told() {
    const PROMPT_HZ: u32 = 500;
    const NO_INPUT_HZ: u32 = 700;
    const INVALID_HZ: u32 = 900;
    let stack = stack_or_skip!();
    let [(_, alice), (carol, carol_phone), (dave, dave_phone)] = three_phones(&stack).await;
    let menu = stack.seed.user("menu", "menu-pw", "1500").await;
    let wav = |name: &str, hz: u32, secs: f32| {
        let path = stack._dir.path().join(name);
        tone_wav(&path, hz as f32, secs);
        path.display().to_string()
    };
    let (prompt, no_input, invalid) = (
        wav("menu.wav", PROMPT_HZ, 1.0),
        wav("no-input.wav", NO_INPUT_HZ, 0.6),
        wav("invalid.wav", INVALID_HZ, 0.6),
    );
    let dial = |to: &str| {
        serde_json::json!({
            "id": format!("dial_{to}"), "name": "Dial", "type": "dial", "destination": to,
            "timeout": 10, "next": "done", "on_error": "done", "on_no_answer": "done",
            "on_busy": "done"
        })
    };
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "patient-menu", "name": "Menu", "version": 1, "initial_node": "answer",
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "menu"},
            "menu": {
                "id": "menu", "name": "Menu", "type": "gather", "num_digits": 1, "timeout": 1,
                "prompt": {"type": "play", "url": prompt},
                "transitions": {"3": format!("dial_{}", carol.extension)},
                "default": "done", "on_timeout": "done",
                "max_attempts": 3,
                "no_input_prompt": {"type": "play", "url": no_input},
                "invalid_prompt": {"type": "play", "url": invalid},
                "on_max_attempts": format!("dial_{}", dave.extension)
            },
            format!("dial_{}", carol.extension): dial(&carol.extension),
            format!("dial_{}", dave.extension): dial(&dave.extension),
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    stack
        .calling(&menu, |s| {
            s.forward_all = Some(CallTarget::Flow("patient-menu".into()))
        })
        .await;

    // A wrong key: told so, asked again, then a right one is taken.
    let call = alice.call(&menu.extension).await.unwrap();
    call.answered(SOON).await.expect("the menu answers");
    assert!(call.media().heard(PROMPT_HZ, SOON).await, "no menu prompt");
    call.media().send_dtmf('9');
    assert!(
        call.media().heard(INVALID_HZ, SOON).await,
        "a wrong key did not play the invalid-choice prompt"
    );
    assert!(
        !call.ended(Duration::from_millis(1500)).await,
        "a wrong key ended the call"
    );
    call.media().send_dtmf('3');
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("the menu asked again did not take the right key", &alice)
            .await;
    };
    at_carol.answer().await.unwrap().hangup().await.unwrap();
    let _ = call.ended(SOON).await;

    // Nothing pressed: told so twice, then on to the last-attempt branch.
    let call = alice.call(&menu.extension).await.unwrap();
    call.answered(SOON).await.expect("the menu answers");
    let asked = Instant::now();
    assert!(
        call.media()
            .heard(NO_INPUT_HZ, Duration::from_secs(8))
            .await,
        "silence did not play the no-input prompt"
    );
    let Some(at_dave) = dave_phone.incoming(Duration::from_secs(15)).await else {
        stack
            .fail("the spent attempts did not go to on_max_attempts", &alice)
            .await;
    };
    // Three prompts and three timeouts, two no-input prompts between:
    // about 7 s. Two attempts would be about 4.6 s.
    let waited = asked.elapsed();
    assert!(
        waited >= Duration::from_secs(6),
        "gave up after {waited:?}: fewer than three attempts"
    );
    assert_eq!(
        call.media().tone_frames(INVALID_HZ),
        0,
        "silence played the invalid-choice prompt"
    );
    at_dave.answer().await.unwrap().hangup().await.unwrap();
    stack.finish().await;
}

/// The caller parks the callee with `*70`: the callee moves to the A
/// slot, and the B slot the caller left must name no one. Still naming the
/// callee, it sent the callee's own audio back between the hold music's
/// packets — choppy music on the D8 server (`docs/DEEP_TESTING.md` D8).
/// The `*70` call is ended by the node, not left for the phone to hang up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_parking_with_the_code_leaves_the_parked_party_only_the_music() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), _] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(alice_call.hold().await.unwrap(), 200);

    let park = alice.call("*70").await.unwrap();
    if let Err(e) = park.answered(SOON).await {
        stack
            .fail(&format!("*70 was not answered: {e}"), &alice)
            .await;
    }
    // The orbit in the answer, for a phone to show: a prompt alone is lost
    // to a speaker that wakes slowly (`docs/DEEP_TESTING.md` D8).
    assert_eq!(park.answer_header("X-FCP-Parked").as_deref(), Some("701"));
    assert!(park.ended(RING_OUT).await, "the *70 call was never ended");
    assert!(alice_call.ended(SOON).await, "Alice's leg was not dropped");
    assert!(
        bob_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await,
        "Bob, parked, hears no music"
    );

    // Bob talks into the park: none of it may come back to him. The music
    // alone is one packet every 20 ms; Bob's own echoed beside it doubled
    // that.
    bob_call.media().heard(1200, Duration::ZERO).await;
    bob_call.media().play_tone(1200.0, 4000);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = bob_call.media().packets_received();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let received = bob_call.media().packets_received() - before;
    assert!(
        received < 200,
        "Bob, parked, got {received} packets in 3 s: his own audio is coming back"
    );
    assert!(
        bob_call.media().tone_frames(1200) == 0,
        "Bob hears himself while parked"
    );

    let retrieve = alice.call("*71701").await.unwrap();
    if let Err(e) = retrieve.answered(SOON).await {
        stack
            .fail(&format!("the retrieve was not answered: {e}"), &alice)
            .await;
    }
    retrieve.media().play_tone(660.0, 3000);
    assert!(
        bob_call.media().heard(660, SOON).await,
        "Bob does not hear Alice after the retrieve"
    );
    bob_call.media().play_tone(880.0, 3000);
    assert!(
        retrieve.media().heard(880, SOON).await,
        "Alice does not hear Bob after the retrieve"
    );
    retrieve.hangup().await.unwrap();
    assert!(bob_call.ended(SOON).await, "Bob's leg outlived the call");
    stack.finish().await;
}

/// The callee parks the caller with `*70`. The callee's leg is To the
/// extension dialed, not the callee's name, so the lookup by name found no
/// call and `*70` was refused 480 — busy on the D8 server's VVX, parking a
/// carrier call it had answered (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_callee_parking_with_the_code_parks_the_caller() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), _] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);

    let park = bob_phone.call("*70").await.unwrap();
    if let Err(e) = park.answered(SOON).await {
        stack
            .fail(&format!("*70 was not answered: {e}"), &bob_phone)
            .await;
    }
    assert!(park.ended(RING_OUT).await, "the *70 call was never ended");
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");
    assert!(
        alice_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await,
        "Alice, parked, hears no music"
    );

    let retrieve = bob_phone.call("*71701").await.unwrap();
    if let Err(e) = retrieve.answered(SOON).await {
        stack
            .fail(&format!("the retrieve was not answered: {e}"), &bob_phone)
            .await;
    }
    retrieve.media().play_tone(660.0, 3000);
    assert!(
        alice_call.media().heard(660, SOON).await,
        "Alice does not hear Bob after the retrieve"
    );
    retrieve.hangup().await.unwrap();
    assert!(
        alice_call.ended(SOON).await,
        "Alice's leg outlived the call"
    );
    stack.finish().await;
}

/// A tenant numbers its own slots (`docs/PARKING.md`, D8 finding 35): with
/// a lot `4320`–`4321` and the park number `4300`, dialing `4300` parks in
/// `4320`, the node's own `701`–`720` is not the tenant's, and `*714320`
/// takes the call back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenants_own_lot_numbers_the_slots() {
    let stack = stack_or_skip!();
    let mut lot =
        fcp_core::parking::ParkingLot::new(stack.seed.tenant_id, "Front desk", "4320", "4321");
    lot.park_number = Some("4300".to_string());
    fcp_core::parking::ParkingLotStore::put_parking_lot(stack.seed.storage(), &lot)
        .await
        .unwrap();
    let [(_, alice), (bob, bob_phone), _] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);

    let park = bob_phone.call("4300").await.unwrap();
    if let Err(e) = park.answered(SOON).await {
        stack
            .fail(
                &format!("the park number was not answered: {e}"),
                &bob_phone,
            )
            .await;
    }
    assert_eq!(
        park.answer_header("X-FCP-Parked").as_deref(),
        Some("4320"),
        "parked in the lot's first slot"
    );
    assert!(park.ended(RING_OUT).await, "the park call was never ended");
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");

    // The node's own range is not this tenant's any more.
    let wrong = bob_phone.call("*71701").await.unwrap();
    assert!(
        wrong.answered(Duration::from_secs(3)).await.is_err(),
        "701 held nothing"
    );

    let retrieve = bob_phone.call("*714320").await.unwrap();
    if let Err(e) = retrieve.answered(SOON).await {
        stack
            .fail(&format!("the retrieve was not answered: {e}"), &bob_phone)
            .await;
    }
    retrieve.media().play_tone(660.0, 3000);
    assert!(
        alice_call.media().heard(660, SOON).await,
        "Alice does not hear Bob after the retrieve"
    );
    retrieve.hangup().await.unwrap();
    stack.finish().await;
}

/// An encrypted caller transfers the callee to a plain phone: the plain
/// phone takes the caller's side, so the node's keys for that side must go
/// — kept, the node would encrypt toward it and refuse its plain packets.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plain_phone_taking_an_encrypted_side_is_heard() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let alice_call = alice
        .call_offering_srtp(
            &bob.extension,
            "1 AES_CM_128_HMAC_SHA1_80 inline:WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz",
        )
        .await
        .unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice).await;
    };
    let bob_call = at_bob.answer().await.unwrap();
    alice_call.answered(SOON).await.expect("answered");
    assert!(
        alice_call
            .answer_sdp()
            .is_some_and(|a| a.contains(" RTP/SAVP ")),
        "Alice's side is encrypted"
    );

    // Alice sends Bob to Carol, who takes Alice's side of the call.
    let refer_to = format!("sip:{}@{}", carol.extension, stack.sip);
    assert_eq!(alice_call.refer(&refer_to).await.unwrap(), 202);
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack.fail("Carol's phone did not ring", &alice).await;
    };
    let carol_call = at_carol.answer().await.unwrap();
    assert!(alice_call.ended(SOON).await, "Alice's leg was not dropped");

    carol_call.media().play_tone(660.0, 3000);
    assert!(
        bob_call.media().heard(660, SOON).await,
        "Bob does not hear Carol"
    );
    bob_call.media().play_tone(440.0, 3000);
    assert!(
        carol_call.media().heard(440, SOON).await,
        "Carol does not hear Bob"
    );
    bob_call.hangup().await.unwrap();
    assert!(carol_call.ended(SOON).await);
    stack.finish().await;
}

/// An encrypted phone taking over a leg is answered encrypted: a key of
/// the node's under the phone's own tag, not plain RTP.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_encrypted_phone_taking_over_a_call_is_answered_encrypted() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (_carol, carol_phone)] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;

    let key = "WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz";
    let replaces = bob_call.replaces().await.unwrap();
    let carol_call = carol_phone
        .call_offering_srtp_with_headers(
            &bob.extension,
            &format!("3 AES_CM_128_HMAC_SHA1_80 inline:{key}"),
            &[("Replaces", replaces.as_str())],
        )
        .await
        .unwrap();
    if let Err(e) = carol_call.answered(SOON).await {
        stack
            .fail(&format!("the Replaces was not answered: {e}"), &carol_phone)
            .await;
    }
    assert!(bob_call.ended(SOON).await, "Bob's leg was not dropped");
    let answer = carol_call.answer_sdp().expect("Carol's answer");
    assert!(
        answer.contains(" RTP/SAVP "),
        "answered in the clear: {answer}"
    );
    let crypto = answer
        .lines()
        .find_map(|l| l.strip_prefix("a=crypto:"))
        .unwrap_or_else(|| panic!("no key: {answer}"));
    assert!(
        crypto.starts_with("3 AES_CM_128_HMAC_SHA1_80 inline:"),
        "{crypto}"
    );
    assert!(!crypto.contains(key), "answered with Carol's own key");

    alice_call.hangup().await.unwrap();
    assert!(carol_call.ended(SOON).await);
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

/// A hold-music prompt of the tenant: a steady `hz` tone.
async fn music_prompt(stack: &Stack, name: &str, hz: f32) -> String {
    use fcp_core::prompt::{Prompt, PromptCategory, PromptStore};
    let path = stack._dir.path().join(format!("{name}.wav"));
    tone_wav(&path, hz, 4.0);
    let prompt = Prompt::new_file(stack.seed.tenant_id, name, PromptCategory::Moh, "en", path);
    let id = prompt.id.to_string();
    fcp_storage_postgres::prompt::PostgresPromptStore::new(stack.seed.storage().pool().clone())
        .put_prompt(prompt)
        .await
        .expect("seed prompt");
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_numbers_hold_music_plays_to_its_callers_unless_the_holder_chose_their_own() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let sales = music_prompt(&stack, "sales-music", 700.0).await;
    let own = music_prompt(&stack, "bobs-music", 1100.0).await;

    // Bob's number plays the sales music.
    scheduled_number(&stack, &bob, "+15125550811", None, None, None).await;
    let storage = stack.seed.storage();
    let number = PhoneNumberStore::get_phone_number_by_e164(storage, "+15125550811")
        .await
        .unwrap()
        .unwrap();
    let mut binding = storage
        .get_line_binding_by_phone_number(&number.id)
        .await
        .unwrap()
        .unwrap();
    binding.hold_music = Some(sales.clone());
    storage.put_line_binding(binding).await.unwrap();

    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    // Alice calls the number; Bob holds her: the number's music, not the
    // built-in.
    let call = alice_phone.call("+15125550811").await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack
            .fail("the number did not ring Bob", &alice_phone)
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    if !call.media().heard(700, SOON).await {
        stack
            .fail("Alice hears no music of the number", &alice_phone)
            .await;
    }
    assert!(
        !call
            .media()
            .heard(DEFAULT_MUSIC_HZ, Duration::from_millis(500))
            .await,
        "the built-in music played over the number's"
    );
    call.hangup().await.unwrap();

    // Bob chooses his own: it wins over the number's.
    stack
        .calling(&bob, |s| s.hold_music = Some(own.clone()))
        .await;
    let alice_phone = stack.phone(&alice).await;
    let call = alice_phone.call("+15125550811").await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack
            .fail("the number did not ring Bob again", &alice_phone)
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    if !call.media().heard(1100, SOON).await {
        stack
            .fail("Alice does not hear Bob's own music", &alice_phone)
            .await;
    }
    assert!(
        !call.media().heard(700, Duration::from_millis(500)).await,
        "the number's music played over Bob's own"
    );
    call.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenants_chosen_hold_music_or_silence_plays_to_every_held_caller() {
    use fcp_core::prompt::{Prompt, PromptCategory, PromptStore};
    use fcp_core::storage::TenantStore;
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    // The tenant's hold-music prompt (700 Hz), which plays when the tenant
    // chooses nothing, and the one it chooses: a greeting at 1100 Hz.
    music_prompt(&stack, "hold-music", 700.0).await;
    let path = stack._dir.path().join("chosen.wav");
    tone_wav(&path, 1100.0, 4.0);
    let chosen = Prompt::new_file(
        stack.seed.tenant_id,
        "chosen",
        PromptCategory::Greeting,
        "en",
        path,
    );
    let chosen_id = chosen.id.to_string();
    fcp_storage_postgres::prompt::PostgresPromptStore::new(stack.seed.storage().pool().clone())
        .put_prompt(chosen)
        .await
        .expect("seed prompt");
    let choose = |music: &'static str, id: String| {
        let storage = stack.seed.storage();
        let tenant_id = stack.seed.tenant_id;
        async move {
            let mut tenant = storage.get_tenant(&tenant_id).await.unwrap().unwrap();
            tenant.calling.hold_music = Some(if music.is_empty() {
                id
            } else {
                music.to_string()
            });
            storage.put_tenant(tenant).await.unwrap();
        }
    };

    // The tenant chooses its greeting: Alice hears it, not the hold-music
    // prompt.
    choose("", chosen_id.clone()).await;
    let bob_phone = stack.phone(&bob).await;
    let alice_phone = stack.phone(&alice).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    if !alice_call.media().heard(1100, SOON).await {
        stack
            .fail("Alice hears no music the tenant chose", &alice_phone)
            .await;
    }
    assert!(
        !alice_call
            .media()
            .heard(700, Duration::from_millis(500))
            .await,
        "the tenant's first hold-music prompt played over its choice"
    );
    alice_call.hangup().await.unwrap();

    // The tenant chooses silence: neither its prompts nor the built-in.
    choose(fcp_core::calling::HOLD_MUSIC_NONE, String::new()).await;
    let alice_phone = stack.phone(&alice).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    let mut heard = false;
    for hz in [1100, 700, DEFAULT_MUSIC_HZ] {
        heard |= alice_call
            .media()
            .heard(hz, Duration::from_millis(700))
            .await;
    }
    if heard {
        stack
            .fail(
                "Alice hears music though the tenant chose silence",
                &alice_phone,
            )
            .await;
    }
    assert_eq!(bob_call.resume().await.unwrap(), 200);
    alice_call.hangup().await.unwrap();

    // Bob's own choice still comes first.
    stack
        .calling(&bob, |s| s.hold_music = Some(chosen_id.clone()))
        .await;
    let alice_phone = stack.phone(&alice).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    if !alice_call.media().heard(1100, SOON).await {
        stack
            .fail("Bob's own music lost to the tenant's silence", &alice_phone)
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
    dial_code(&stack, &bob_phone, "*57").await;
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

// ─── Schedules (docs/ENTERPRISE_CALLING.md §3.5) ─────────────────────────

/// A schedule that is never open (no hours), one that always is, and one
/// on a holiday that spans today wherever the test runs.
async fn schedules(stack: &Stack) -> (fcp_core::Schedule, fcp_core::Schedule, fcp_core::Schedule) {
    use chrono::{Duration as Days, NaiveTime, Utc, Weekday};
    use fcp_core::{Holiday, Schedule, ScheduleStore, WeeklyWindow};
    let tenant = stack.seed.tenant_id;
    let closed = Schedule::new(tenant, "never open", "UTC");
    let mut open = Schedule::new(tenant, "always open", "UTC");
    let every_day = vec![
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];
    open.weekly.push(WeeklyWindow {
        days: every_day.clone(),
        start: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
    });
    let mut holiday = open.clone();
    holiday.id = fcp_core::ScheduleId::new();
    holiday.name = "holiday".into();
    let today = Utc::now().date_naive();
    holiday.holidays.push(Holiday {
        name: "today".into(),
        from: today - Days::days(1),
        to: Some(today + Days::days(1)),
    });
    for s in [&closed, &open, &holiday] {
        stack
            .seed
            .storage()
            .put_schedule(s.clone())
            .await
            .expect("seed schedule");
    }
    (closed, open, holiday)
}

/// A number bound to `owner`, with the schedule fields given.
async fn scheduled_number(
    stack: &Stack,
    owner: &SeededUser,
    e164: &str,
    schedule: Option<&fcp_core::Schedule>,
    closed: Option<CallTarget>,
    holiday: Option<CallTarget>,
) {
    let storage = stack.seed.storage();
    let user = fcp_core::storage::CredentialStore::get_user_in_tenant(
        storage,
        &owner.username,
        &stack.seed.tenant_id,
    )
    .await
    .unwrap()
    .unwrap();
    let devices = fcp_core::storage::DeviceStore::list_devices_by_user(storage, &user.id)
        .await
        .unwrap();
    let number = fcp_core::PhoneNumber::new(e164.to_string(), stack.seed.tenant_id).unwrap();
    let mut binding = fcp_core::LineBinding::new(
        number.id,
        Some(user.id),
        fcp_core::BindingType::User,
        stack.seed.tenant_id,
    );
    for d in devices {
        binding.add_device(fcp_core::DeviceBinding::new(d.id));
    }
    binding.schedule_id = schedule.map(|s| s.id);
    binding.closed_target = closed;
    binding.holiday_target = holiday;
    storage.put_phone_number(number).await.unwrap();
    storage.put_line_binding(binding).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_number_goes_where_it_says_and_an_open_one_rings() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let (closed, open, holiday) = schedules(&stack).await;
    let carol_ext = CallTarget::Extension(carol.extension.clone());
    scheduled_number(
        &stack,
        &bob,
        "+15125550801",
        Some(&closed),
        Some(carol_ext.clone()),
        None,
    )
    .await;
    scheduled_number(
        &stack,
        &bob,
        "+15125550802",
        Some(&open),
        Some(carol_ext.clone()),
        None,
    )
    .await;
    scheduled_number(
        &stack,
        &bob,
        "+15125550803",
        Some(&holiday),
        Some(carol_ext.clone()),
        Some(CallTarget::Voicemail),
    )
    .await;
    stack.mailbox(&bob, "+15125550899").await;
    let voicemail = stack.voicemail_server().await;

    // Closed: Carol, not Bob.
    let call = alice.call("+15125550801").await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("the closed number did not go to Carol", &alice)
            .await;
    };
    assert!(
        bob_phone
            .incoming(Duration::from_millis(300))
            .await
            .is_none(),
        "Bob rang while closed"
    );
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    answered.hangup().await.unwrap();

    // Open: Bob.
    let call = alice.call("+15125550802").await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack.fail("the open number did not ring Bob", &alice).await;
    };
    let answered = at_bob.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    answered.hangup().await.unwrap();

    // A holiday: its own target, voicemail, and the reason says so.
    let call = alice.call("+15125550803").await.unwrap();
    let Some(at_voicemail) = voicemail.incoming(SOON).await else {
        stack
            .fail("the holiday did not go to voicemail", &alice)
            .await;
    };
    assert!(
        at_voicemail.request_uri().contains("reason=holiday"),
        "{}",
        at_voicemail.request_uri()
    );
    let answered = at_voicemail.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_number_can_run_a_flow_and_a_user_forwards_after_hours() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let (closed, _, _) = schedules(&stack).await;

    // The closed number runs a flow that answers and hangs up.
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "closed-greeting",
        "name": "Closed",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": stack.seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "wait"},
            "wait": {"id": "wait", "name": "Wait", "type": "pause", "duration": 1, "next": "done"},
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .unwrap();
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .unwrap();
    scheduled_number(
        &stack,
        &bob,
        "+15125550811",
        Some(&closed),
        Some(CallTarget::Flow("closed-greeting".into())),
        None,
    )
    .await;
    let call = alice.call("+15125550811").await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(
                &format!("the closed number's flow did not answer: {e}"),
                &alice,
            )
            .await;
    }
    assert!(call.ended(SOON).await, "the flow did not hang up");
    assert!(
        bob_phone
            .incoming(Duration::from_millis(300))
            .await
            .is_none(),
        "Bob rang"
    );

    // Bob's own hours are never open: his calls go to Carol.
    stack
        .calling(&bob, |s| {
            s.after_hours_schedule_id = Some(closed.id);
            s.after_hours_forward = Some(CallTarget::Extension(carol.extension.clone()));
        })
        .await;
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Bob's after-hours forward did not reach Carol", &alice)
            .await;
    };
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flow_branches_on_a_schedule() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), _] = three_phones(&stack).await;
    let (closed, open, _) = schedules(&stack).await;
    for (id, schedule, number) in [
        ("hours-open", &open, "+15125550821"),
        ("hours-closed", &closed, "+15125550822"),
    ] {
        let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
            "id": id,
            "name": id,
            "version": 1,
            "initial_node": "answer",
            "enabled": true,
            "tenant_id": stack.seed.tenant_id.to_string(),
            "nodes": {
                "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "hours"},
                "hours": {
                    "id": "hours", "name": "Hours", "type": "schedule",
                    "schedule_id": schedule.id.to_string(),
                    "open": "dial", "closed": "done"
                },
                "dial": {
                    "id": "dial", "name": "Dial Bob", "type": "dial",
                    "destination": bob.extension, "timeout": 10, "next": "done",
                    "on_error": "done", "on_no_answer": "done", "on_busy": "done"
                },
                "done": {"id": "done", "name": "Hang up", "type": "hangup"}
            }
        }))
        .unwrap();
        fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
            .await
            .unwrap();
        let number = fcp_core::PhoneNumber::new(number.to_string(), stack.seed.tenant_id).unwrap();
        let mut binding = fcp_core::LineBinding::new(
            number.id,
            None,
            fcp_core::BindingType::Ivr,
            stack.seed.tenant_id,
        );
        binding.flow_id = Some(id.to_string());
        stack.seed.storage().put_phone_number(number).await.unwrap();
        stack
            .seed
            .storage()
            .put_line_binding(binding)
            .await
            .unwrap();
    }

    // Open: the flow dials Bob.
    let call = alice.call("+15125550821").await.unwrap();
    call.answered(SOON).await.expect("the flow answers");
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack.fail("the open branch did not dial Bob", &alice).await;
    };
    let answered = at_bob.answer().await.unwrap();
    call.hangup().await.unwrap();
    let _ = answered.ended(SOON).await;

    // Closed: the flow hangs up without dialing. It answers and hangs up at
    // once, so the phone may see the hangup before the answer settles.
    let call = alice.call("+15125550822").await.unwrap();
    let _ = call.answered(SOON).await;
    if !call.ended(SOON).await {
        let why = format!("the closed branch did not hang up: {:?}", call.phase());
        stack.fail(&why, &alice).await;
    }
    assert!(
        bob_phone
            .incoming(Duration::from_millis(300))
            .await
            .is_none(),
        "Bob was dialed while closed"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_queue_sends_its_callers_on() {
    let Some(stack) = Stack::start_with("[queue]\nenabled = true\n").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let [(_, alice), _, (carol, carol_phone)] = three_phones(&stack).await;
    let (closed, _, _) = schedules(&stack).await;
    let mut queue = fcp_core::QueueConfigRecord::new(
        "support".to_string(),
        "7100".to_string(),
        stack.seed.tenant_id,
    );
    queue.schedule_id = Some(closed.id);
    queue.closed_target = Some(CallTarget::Extension(carol.extension.clone()));
    fcp_core::storage::QueueConfigStore::put_queue_config(stack.seed.storage(), queue)
        .await
        .unwrap();
    let (code, body) = fcp_testkit::smoke::http_post(&loopback(stack.http, "/queues/reload"), "")
        .await
        .expect("reload queues");
    assert_eq!(code, 200, "{body}");

    let call = alice.call("7100").await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("the closed queue did not send the caller to Carol", &alice)
            .await;
    };
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    answered.hangup().await.unwrap();
    stack.finish().await;
}

/// A queue made on the admin API after the node started is answered with
/// no signal to the node: it read its queues once, at start, and a queue
/// made later was refused until the node was signalled
/// (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queue_made_after_start_is_answered() {
    let Some(stack) = Stack::start_with("[queue]\nenabled = true\nstore_sync_secs = 1\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let [(_, alice), _, _] = three_phones(&stack).await;
    let queue = fcp_core::QueueConfigRecord::new(
        "late".to_string(),
        "7200".to_string(),
        stack.seed.tenant_id,
    );
    fcp_core::storage::QueueConfigStore::put_queue_config(stack.seed.storage(), queue)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let call = alice.call("7200").await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("the new queue did not answer: {e}"), &alice)
            .await;
    }
    call.hangup().await.unwrap();
    stack.finish().await;
}

// ─── Find-me / follow-me (docs/ENTERPRISE_CALLING.md §3.6) ───────────────

fn find_me(steps: serde_json::Value) -> fcp_core::FindMe {
    serde_json::from_value(serde_json::json!({ "enabled": true, "steps": steps })).expect("find-me")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_me_rings_the_next_step_when_the_phones_do_not_answer() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let steps =
        serde_json::json!([{ "targets": [format!("ext:{}", carol.extension)], "ring_secs": 10 }]);
    stack
        .calling(&bob, |s| {
            s.no_answer_secs = Some(5);
            s.find_me = Some(find_me(steps));
        })
        .await;

    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring first", &alice).await;
    };
    let ringing = at_bob.leave_ringing();
    let Some(at_carol) = carol_phone.incoming(RING_OUT).await else {
        stack
            .fail("the find-me step did not ring Carol", &alice)
            .await;
    };
    assert!(ringing.ended(SOON).await, "Bob's phone kept ringing");
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_confirmed_step_connects_only_on_a_one() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let steps = serde_json::json!([{ "targets": [format!("ext:{}", carol.extension)], "ring_secs": 10, "confirm": true }]);
    stack
        .calling(&bob, |s| {
            s.no_answer_secs = Some(5);
            s.find_me = Some(find_me(steps));
        })
        .await;

    // Carol answers and presses 1: Alice is connected.
    let call = alice.call(&bob.extension).await.unwrap();
    let _ringing = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob rings")
        .leave_ringing();
    let Some(at_carol) = carol_phone.incoming(RING_OUT).await else {
        stack
            .fail("the find-me step did not ring Carol", &alice)
            .await;
    };
    let carol_call = at_carol.answer().await.unwrap();
    assert!(
        call.answered(Duration::from_millis(1500)).await.is_err(),
        "Alice was connected before Carol confirmed"
    );
    carol_call.media().send_dtmf('1');
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("Carol's 1 did not connect Alice: {e}"), &alice)
            .await;
    }
    carol_call.hangup().await.unwrap();
    let _ = call.ended(SOON).await;

    // Carol answers and presses 2: her leg is dropped and Alice is refused.
    let call = alice.call(&bob.extension).await.unwrap();
    let _ringing = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob rings")
        .leave_ringing();
    let Some(at_carol) = carol_phone.incoming(RING_OUT).await else {
        stack
            .fail("the find-me step did not ring Carol again", &alice)
            .await;
    };
    let carol_call = at_carol.answer().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    carol_call.media().send_dtmf('2');
    assert!(
        carol_call.ended(SOON).await,
        "Carol's unconfirmed leg was not dropped"
    );
    assert!(call.failed(SOON).await.is_some(), "Alice was not refused");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_me_rings_even_with_no_phone_of_the_users_up() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let alice_phone = stack.phone(&alice).await;
    let carol_phone = stack.phone(&carol).await;
    let steps = serde_json::json!([{ "targets": ["devices", format!("ext:{}", carol.extension)], "ring_secs": 10 }]);
    stack
        .calling(&bob, |s| s.find_me = Some(find_me(steps)))
        .await;

    let call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail(
                "find-me did not ring Carol for an unregistered Bob",
                &alice_phone,
            )
            .await;
    };
    let answered = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    answered.hangup().await.unwrap();
    stack.finish().await;
}

// ─── Class of service (docs/ENTERPRISE_CALLING.md §3.7) ──────────────────

/// Store `class` and make it `user`'s, keeping their other settings.
async fn give_class(stack: &Stack, user: &SeededUser, class: &fcp_core::ClassOfService) {
    use fcp_core::ClassOfServiceStore;
    stack
        .seed
        .storage()
        .put_class_of_service(class.clone())
        .await
        .expect("store class");
    let id = class.id;
    stack
        .calling(user, |s| s.class_of_service_id = Some(id))
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_limits_the_calls_a_user_is_on() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let mut one = fcp_core::ClassOfService::new(stack.seed.tenant_id, "One at a time");
    one.max_calls = Some(1);
    give_class(&stack, &bob, &one).await;

    // Call waiting is on, but the class allows one call: a second to Bob
    // is busy without ringing.
    let (alice_call, _) = answered(&stack, &alice, &bob, &bob_phone).await;
    let second = carol_phone.call(&bob.extension).await.unwrap();
    match second.failed(SOON).await {
        Some(486) => {}
        other => {
            stack
                .fail(
                    &format!("the class's limit did not refuse: {other:?}"),
                    &carol_phone,
                )
                .await
        }
    }
    assert!(
        bob_phone.incoming(QUIET).await.is_none(),
        "Bob's phone rang past his class's limit"
    );

    // And Bob cannot place one either.
    let placed = bob_phone.call(&carol.extension).await.unwrap();
    assert_eq!(placed.failed(SOON).await, Some(403));
    assert!(carol_phone.incoming(QUIET).await.is_none());

    alice_call.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forward_the_class_forbids_is_not_followed() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, _bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let mut internal = fcp_core::ClassOfService::new(stack.seed.tenant_id, "Internal");
    internal.allowed = vec![fcp_core::DestinationClass::Internal];
    give_class(&stack, &bob, &internal).await;
    let class_id = internal.id;
    stack
        .calling(&bob, |s| {
            s.class_of_service_id = Some(class_id);
            s.forward_all = Some(CallTarget::Number("+442079460000".into()));
        })
        .await;

    // Bob may not call London, so his forward does not either.
    let call = alice.call(&bob.extension).await.unwrap();
    assert_eq!(call.failed(SOON).await, Some(480));

    // An extension is internal: the same class forwards there.
    stack
        .calling(&bob, |s| {
            s.class_of_service_id = Some(class_id);
            s.forward_all = Some(CallTarget::Extension(carol.extension.clone()));
        })
        .await;
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("the internal forward did not ring Carol", &alice)
            .await;
    };
    let leg = at_carol.answer().await.unwrap();
    call.answered(SOON).await.expect("Alice reaches Carol");
    leg.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_number_screens_anonymous_and_unlisted_callers() {
    use fcp_core::storage::PhoneNumberStore;
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), _] = three_phones(&stack).await;
    scheduled_number(&stack, &bob, "+15125550901", None, None, None).await;
    let storage = stack.seed.storage();
    let mut number = storage
        .get_phone_number_by_e164("+15125550901")
        .await
        .unwrap()
        .unwrap();
    number.block_anonymous = true;
    storage.put_phone_number(number.clone()).await.unwrap();

    // A withheld caller is refused (RFC 5079); a known one rings.
    let anonymous = alice
        .call_with_headers("+15125550901", &[("Privacy", "id")])
        .await
        .unwrap();
    assert_eq!(anonymous.failed(SOON).await, Some(433));
    let known = alice.call("+15125550901").await.unwrap();
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack
            .fail("an unscreened caller did not ring Bob", &alice)
            .await;
    };
    at_bob.reject(486, "Busy Here").await.unwrap();
    let _ = known.failed(SOON).await;

    // An allow list lets in only who it names.
    number.allow_list = vec!["+15125550000".into()];
    storage.put_phone_number(number).await.unwrap();
    let unlisted = alice.call("+15125550901").await.unwrap();
    assert_eq!(unlisted.failed(SOON).await, Some(603));
    assert!(bob_phone.incoming(QUIET).await.is_none());
    stack.finish().await;
}

// ─── Hoteling (docs/ENTERPRISE_CALLING.md §3.8) ──────────────────────────

const HOST_PASSWORD: &str = "lobby-phone-pw";
const BOB_PIN: &str = "4321";

/// A shared phone with credentials of its own and no owner, users may log
/// in on.
async fn host_device(stack: &Stack, name: &str) -> fcp_core::Device {
    use fcp_core::storage::DeviceStore;
    let mut device = fcp_core::Device::new(
        format!("{name} phone"),
        fcp_core::DeviceType::DeskPhone,
        stack.seed.tenant_id,
    );
    device.sip_uri = Some(format!("sip:{name}@{REALM}"));
    device.sip_username = Some(name.to_string());
    device.sip_ha1_hash = Some(fcp_core::storage::password::compute_ha1(
        name,
        REALM,
        HOST_PASSWORD,
    ));
    device.hoteling_host = true;
    stack
        .seed
        .storage()
        .put_device(device.clone())
        .await
        .expect("store host");
    device
}

/// Give a user a PIN, returning their id.
async fn set_pin(stack: &Stack, user: &SeededUser, pin: &str) -> fcp_core::UserId {
    use fcp_core::storage::CredentialStore;
    let storage = stack.seed.storage();
    let mut row = storage
        .get_user_in_tenant(&user.username, &stack.seed.tenant_id)
        .await
        .expect("read user")
        .expect("seeded user");
    row.pin = Some(fcp_core::storage::password::hash_pin(pin).expect("pin hashes"));
    let id = row.id;
    storage.put_user(row).await.expect("store user");
    id
}

/// Key `digits` into the call once the node is listening for them.
async fn key(call: &fcp_testkit::Call, digits: &str) {
    tokio::time::sleep(Duration::from_millis(800)).await;
    for digit in digits.chars() {
        call.media().send_dtmf(digit);
    }
}

/// Dial `*88` on the host and log in with `extension` and `pin`, as many
/// times as `attempts` says; the call as the node answered it.
async fn log_in(stack: &Stack, host: &Phone, attempts: &[(&str, &str)]) -> fcp_testkit::Call {
    let call = host.call("*88").await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("*88 was not answered: {e}"), host)
            .await;
    }
    for (i, (extension, pin)) in attempts.iter().enumerate() {
        if i > 0 {
            // A refused attempt is checked and told before the node asks
            // again, as the caller would hear.
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        key(&call, &format!("{extension}#")).await;
        key(&call, &format!("{pin}#")).await;
    }
    call
}

/// Wait for a `check-sync` NOTIFY on `phone`.
async fn told_to_resync(phone: &Phone) -> bool {
    phone
        .expect(
            RING_OUT,
            |e| matches!(e, PhoneEvent::Notify { event, .. } if event.starts_with("check-sync")),
        )
        .await
        .is_some()
}

async fn session_of(stack: &Stack, user: &fcp_core::UserId) -> Option<fcp_core::HotelingSession> {
    use fcp_core::HotelingStore;
    stack
        .seed
        .storage()
        .hoteling_session_for_user(user)
        .await
        .expect("read session")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guest_logs_in_on_a_host_and_the_host_is_theirs() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_id = set_pin(&stack, &bob, BOB_PIN).await;
    let mut internal = fcp_core::ClassOfService::new(stack.seed.tenant_id, "Internal");
    internal.allowed = vec![fcp_core::DestinationClass::Internal];
    give_class(&stack, &bob, &internal).await;
    host_device(&stack, "lobby").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    // The host registers with its own credentials.
    let host = Phone::new("lobby", HOST_PASSWORD, REALM, stack.sip)
        .await
        .expect("host binds");
    assert_eq!(host.register().await.unwrap(), 200);

    // Nobody is logged in: a call to Bob rings his own phone only.
    let call = alice_phone.call(&bob.extension).await.unwrap();
    bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    assert!(
        host.incoming(QUIET).await.is_none(),
        "the host rang for Bob before he logged in"
    );
    call.hangup().await.ok();
    let _ = call.ended(SOON).await;

    // A wrong PIN, then the right one.
    let login = log_in(&stack, &host, &[("1002", "9999"), ("1002", BOB_PIN)]).await;
    if !told_to_resync(&host).await {
        stack
            .fail("the host was not told to re-provision", &host)
            .await;
    }
    if !login.ended(SOON).await {
        stack.fail("the login call was not ended", &host).await;
    }
    let session = session_of(&stack, &bob_id).await.expect("Bob is logged in");
    assert!(session.expires_at > std::time::SystemTime::now() + Duration::from_secs(11 * 3600));

    // Bob's calls ring the host, as well as his own phone.
    let call = alice_phone.call(&bob.extension).await.unwrap();
    let _at_bob = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    let Some(at_host) = host.incoming(SOON).await else {
        stack
            .fail("the host did not ring for its guest", &alice_phone)
            .await;
    };
    let answered = at_host.answer().await.unwrap();
    call.answered(SOON)
        .await
        .expect("Alice reaches Bob at the host");
    answered.hangup().await.unwrap();
    let _ = call.ended(SOON).await;

    // Calls from the host are Bob's: his class refuses a national number,
    // and his feature codes change his settings.
    let placed = host.call("2025550100").await.unwrap();
    assert_eq!(
        placed.failed(SOON).await,
        Some(403),
        "Bob's class did not apply at the host"
    );
    let dnd = host.call("*78").await.unwrap();
    let _ = dnd.ended(SOON).await;
    let settings = fcp_core::CallingStore::get_calling_settings(
        stack.seed.storage(),
        &stack.seed.tenant_id,
        &bob_id,
    )
    .await
    .expect("read settings")
    .expect("Bob's settings");
    assert!(
        settings.dnd,
        "*78 at the host did not set Bob's do not disturb"
    );
    stack.calling(&bob, |s| s.dnd = false).await;
    give_class(&stack, &bob, &internal).await;

    // *89: the host is nobody's guest again.
    let logout = host.call("*89").await.unwrap();
    let _ = logout.ended(SOON).await;
    if !told_to_resync(&host).await {
        stack
            .fail("the host was not told to re-provision on logout", &host)
            .await;
    }
    assert!(
        session_of(&stack, &bob_id).await.is_none(),
        "*89 left Bob logged in"
    );
    let call = alice_phone.call(&bob.extension).await.unwrap();
    bob_phone.incoming(SOON).await.expect("Bob's phone rings");
    assert!(
        host.incoming(QUIET).await.is_none(),
        "the host rang for Bob after he logged out"
    );
    call.hangup().await.ok();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hoteling_session_runs_out() {
    let Some(stack) = Stack::start_with("[pbx]\nhoteling_sweep_secs = 1\n").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_id = set_pin(&stack, &bob, BOB_PIN).await;
    host_device(&stack, "lobby").await;
    let host = Phone::new("lobby", HOST_PASSWORD, REALM, stack.sip)
        .await
        .expect("host binds");
    assert_eq!(host.register().await.unwrap(), 200);
    let login = log_in(&stack, &host, &[("1002", BOB_PIN)]).await;
    assert!(told_to_resync(&host).await, "no check-sync at login");
    let _ = login.ended(SOON).await;
    let session = session_of(&stack, &bob_id).await.expect("Bob is logged in");

    stack
        .seed
        .execute(&format!(
            "UPDATE hoteling_sessions SET expires_at = NOW() - INTERVAL '1 second' WHERE id = '{}'",
            session.id
        ))
        .await;
    if !told_to_resync(&host).await {
        stack
            .fail("the host was not told when the session ran out", &host)
            .await;
    }
    let ended = fcp_core::HotelingStore::get_hoteling_session(stack.seed.storage(), &session.id)
        .await
        .expect("read session")
        .expect("the session is kept");
    assert_eq!(ended.ended_by, Some(fcp_core::SessionEnd::Expired));
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn credentials_register_only_what_is_theirs() {
    use fcp_core::storage::DeviceStore;
    let stack = stack_or_skip!();
    stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    host_device(&stack, "lobby").await;
    let bob_id = set_pin(&stack, &bob, BOB_PIN).await;
    let mut desk = fcp_core::Device::new(
        "Bob's desk".to_string(),
        fcp_core::DeviceType::DeskPhone,
        stack.seed.tenant_id,
    );
    desk.owner_user_id = Some(bob_id);
    desk.sip_uri = Some(format!("sip:bob-desk@{REALM}"));
    stack.seed.storage().put_device(desk).await.expect("desk");

    let register = |aor: &'static str, auth: &'static str, pw: &'static str| {
        let sip = stack.sip;
        async move {
            let phone = Phone::with_credentials(aor, auth, pw, REALM, sip)
                .await
                .expect("phone binds");
            phone.register().await.unwrap()
        }
    };
    // A user's own address, their extension, a device they own.
    assert_eq!(register("bob", "bob", "bob-pw").await, 200);
    assert_eq!(register("1002", "bob", "bob-pw").await, 200);
    assert_eq!(register("bob-desk", "bob", "bob-pw").await, 200);
    // Somebody else's address, a host that is not theirs.
    assert_eq!(register("alice", "bob", "bob-pw").await, 403);
    assert_eq!(register("lobby", "bob", "bob-pw").await, 403);
    // The host's own credentials register the host, and nobody else.
    assert_eq!(register("lobby", "lobby", HOST_PASSWORD).await, 200);
    assert_eq!(register("alice", "lobby", HOST_PASSWORD).await, 403);
    // A wrong password is still challenged, not refused.
    assert_eq!(register("lobby", "lobby", "wrong-password").await, 401);
    stack.finish().await;
}

// ---------------------------------------------------------------------------
// Shared call appearance (docs/ENTERPRISE_CALLING.md §3.9)
// ---------------------------------------------------------------------------

/// A shared line's number.
const LINE: &str = "+15550188";
const DESK_PASSWORD: &str = "desk-phone-pw";
/// What a phone puts in `Call-Info` to name an appearance.
fn appearance(index: u8) -> String {
    format!("<sip:127.0.0.1>;appearance-index={index}")
}

/// A shared line owned by `owner` with `max` appearances, on a device of
/// its own with its own credentials for each of `desks`.
async fn shared_line(stack: &Stack, owner: &SeededUser, desks: &[&str], max: u8) {
    use fcp_core::storage::{CredentialStore, DeviceStore};
    let storage = stack.seed.storage();
    let owner = storage
        .get_user_in_tenant(&owner.username, &stack.seed.tenant_id)
        .await
        .expect("read owner")
        .expect("seeded owner");
    let number =
        fcp_core::PhoneNumber::new(LINE.to_string(), stack.seed.tenant_id).expect("number");
    let mut binding = fcp_core::LineBinding::new(
        number.id,
        Some(owner.id),
        fcp_core::BindingType::User,
        stack.seed.tenant_id,
    );
    binding.shared = true;
    binding.max_appearances = max;
    for desk in desks {
        let mut device = fcp_core::Device::new(
            format!("{desk} phone"),
            fcp_core::DeviceType::DeskPhone,
            stack.seed.tenant_id,
        );
        device.sip_uri = Some(format!("sip:{desk}@{REALM}"));
        device.sip_username = Some(desk.to_string());
        device.sip_ha1_hash = Some(fcp_core::storage::password::compute_ha1(
            desk,
            REALM,
            DESK_PASSWORD,
        ));
        binding.add_device(fcp_core::DeviceBinding::with_settings(
            device.id, 1, 2, true,
        ));
        storage.put_device(device).await.expect("store desk");
    }
    storage.put_phone_number(number).await.expect("number");
    storage.put_line_binding(binding).await.expect("binding");
}

/// The line's account on `desk`, registered with the desk's credentials.
async fn line_phone(stack: &Stack, desk: &str) -> Phone {
    let phone = Phone::with_credentials(LINE, desk, DESK_PASSWORD, REALM, stack.sip)
        .await
        .expect("phone binds");
    assert_eq!(
        phone.register().await.unwrap(),
        200,
        "{desk} registers the line"
    );
    phone
}

/// Wait until `phone` is sent lamps reading `want`: each index and state
/// in order, `None` for the `*` idle entry that ends every list.
async fn lamps(phone: &Phone, want: &[(Option<u8>, &str)]) -> bool {
    let want: Vec<(Option<u8>, String)> = want.iter().map(|(i, s)| (*i, s.to_string())).collect();
    phone
        .expect(SOON, |e| {
            matches!(e, PhoneEvent::Notify { event, call_info: Some(info), .. }
                if event.starts_with("call-info")
                    && fcp_core::sca::appearance_states(info) == want)
        })
        .await
        .is_some()
}

const IDLE: (Option<u8>, &str) = (None, "idle");

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shared_line_rings_every_phone_and_any_picks_up_a_held_call() {
    let Some(stack) = Stack::start_configured("", "auth_invites = true").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    shared_line(&stack, &alice, &["desk-a", "desk-b"], 2).await;
    let a = line_phone(&stack, "desk-a").await;
    let b = line_phone(&stack, "desk-b").await;
    let carol_phone = stack.phone(&carol).await;

    // Both watch the line; neither may be watched by someone who is not it.
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(
        alice_phone
            .subscribe(LINE, "call-info", 3600, None)
            .await
            .unwrap()
            .code,
        403,
        "a subscriber that is not the line is refused"
    );
    for phone in [&a, &b] {
        assert_eq!(
            phone
                .subscribe(LINE, "call-info", 3600, None)
                .await
                .unwrap()
                .code,
            200
        );
        if !lamps(phone, &[IDLE]).await {
            stack
                .fail("a watcher was not sent the idle line", phone)
                .await;
        }
    }

    // A call to the line rings both phones at its first appearance.
    let call = carol_phone.call(LINE).await.unwrap();
    let Some(ring_a) = a.incoming(SOON).await else {
        stack.fail("the line's first phone did not ring", &a).await;
    };
    let Some(_ring_b) = b.incoming(SOON).await else {
        stack.fail("the line's second phone did not ring", &b).await;
    };
    if !lamps(&b, &[(Some(1), "alerting"), IDLE]).await {
        stack
            .fail("the line's phones were not shown it alerting", &b)
            .await;
    }
    let a_call = ring_a.answer().await.unwrap();
    call.answered(SOON).await.expect("the caller is connected");
    if !lamps(&b, &[(Some(1), "active"), IDLE]).await {
        stack
            .fail("the other phone was not shown the call active", &b)
            .await;
    }

    // The phone that answered holds; every phone sees it held.
    assert_eq!(a_call.hold().await.unwrap(), 200);
    if !lamps(&b, &[(Some(1), "held"), IDLE]).await {
        stack
            .fail("the other phone was not shown the call held", &b)
            .await;
    }

    // The other phone picks it up: it has the caller, the first is let go.
    let b_call = b
        .call_with_headers(LINE, &[("Call-Info", &appearance(1))])
        .await
        .unwrap();
    if let Err(e) = b_call.answered(SOON).await {
        stack
            .fail(&format!("the retrieve was not answered: {e}"), &b)
            .await;
    }
    assert!(
        a_call.ended(SOON).await,
        "the phone that held the call is let go"
    );
    if !lamps(&a, &[(Some(1), "active"), IDLE]).await {
        stack
            .fail("the first phone was not shown the call active again", &a)
            .await;
    }
    assert!(!call.ended(QUIET).await, "the caller stays on the call");

    // A retrieve of a call that is not held is refused.
    let again = a
        .call_with_headers(LINE, &[("Call-Info", &appearance(1))])
        .await
        .unwrap();
    assert_eq!(again.failed(SOON).await, Some(403));

    // The call ends; the line goes idle on every phone.
    b_call.hangup().await.unwrap();
    assert!(
        call.ended(SOON).await,
        "the caller's call ends with the line's"
    );
    if !lamps(&a, &[IDLE]).await {
        stack.fail("the line did not go idle", &a).await;
    }
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_seizes_an_appearance_and_a_full_line_is_busy() {
    let Some(stack) = Stack::start_configured("", "auth_invites = true").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let dave = stack.seed.user("dave", "dave-pw", "1004").await;
    shared_line(&stack, &alice, &["desk-a", "desk-b"], 2).await;
    let a = line_phone(&stack, "desk-a").await;
    let b = line_phone(&stack, "desk-b").await;
    let carol_phone = stack.phone(&carol).await;
    let dave_phone = stack.phone(&dave).await;
    assert_eq!(
        b.subscribe(LINE, "call-info", 3600, None)
            .await
            .unwrap()
            .code,
        200
    );
    assert!(lamps(&b, &[IDLE]).await);

    // The first phone seizes the second appearance; the other phone sees
    // it seized and cannot have it.
    let seize = a
        .subscribe(LINE, "line-seize", 15, Some(&appearance(2)))
        .await
        .unwrap();
    assert_eq!(seize.code, 200);
    let told = a
        .expect(SOON, |e| {
            matches!(e, PhoneEvent::Notify { event, call_info: Some(info), .. }
            if event.starts_with("line-seize")
                && fcp_core::sca::appearance_states(info) == vec![(Some(2), "seized".to_string())])
        })
        .await;
    assert!(told.is_some(), "the seizing phone is told its appearance");
    if !lamps(&b, &[(Some(2), "seized"), IDLE]).await {
        stack
            .fail("the other phone was not shown the seize", &b)
            .await;
    }
    assert_eq!(
        b.subscribe(LINE, "line-seize", 15, Some(&appearance(2)))
            .await
            .unwrap()
            .code,
        480,
        "a seized appearance is not the other phone's"
    );

    // The call from it takes that appearance; the far party is not told.
    let out = a
        .call_with_headers("1003", &[("Call-Info", &appearance(2))])
        .await
        .unwrap();
    let Some(ringing) = carol_phone.incoming(SOON).await else {
        stack
            .fail("the call from the line did not ring", &carol_phone)
            .await;
    };
    assert!(
        ringing
            .headers_named("Call-Info")
            .iter()
            .all(|v| !v.contains("appearance-index")),
        "the line's appearance stays the line's"
    );
    if !lamps(&b, &[(Some(2), "progressing"), IDLE]).await {
        stack
            .fail("the other phone was not shown the call progressing", &b)
            .await;
    }
    let carol_call = ringing.answer().await.unwrap();
    out.answered(SOON)
        .await
        .expect("the call from the line is answered");
    if !lamps(&b, &[(Some(2), "active"), IDLE]).await {
        stack
            .fail("the other phone was not shown the call active", &b)
            .await;
    }

    // A call to the line takes the one appearance left; the next is busy.
    let first = carol_phone.call(LINE).await.unwrap();
    if !lamps(&b, &[(Some(1), "alerting"), (Some(2), "active"), IDLE]).await {
        stack
            .fail("the call to the line was not shown alerting", &b)
            .await;
    }
    let second = dave_phone.call(LINE).await.unwrap();
    assert_eq!(
        second.failed(SOON).await,
        Some(486),
        "every appearance is in use"
    );

    // The free phone takes the call ringing on the line; all hang up.
    let Some(ring) = b.incoming(SOON).await else {
        stack.fail("the free phone did not ring", &b).await;
    };
    let _b_call = ring.answer().await.unwrap();
    first
        .answered(SOON)
        .await
        .expect("the call to the line is answered");
    first.hangup().await.unwrap();
    out.hangup().await.unwrap();
    assert!(carol_call.ended(SOON).await);
    if !lamps(&b, &[IDLE]).await {
        stack.fail("the line did not go idle", &b).await;
    }
    stack.finish().await;
}

// ---------------------------------------------------------------------------
// Executive-assistant (docs/ENTERPRISE_CALLING.md §3.10)
// ---------------------------------------------------------------------------

/// Make `executive` an executive whose `assistants` take the calls
/// `filter` selects.
async fn make_executive(
    stack: &Stack,
    executive: &SeededUser,
    assistants: &[&SeededUser],
    filter: fcp_core::ExecutiveFilter,
) {
    use fcp_core::storage::CredentialStore;
    let storage = stack.seed.storage();
    let id_of = |user: &SeededUser| {
        let username = user.username.clone();
        async move {
            storage
                .get_user_in_tenant(&username, &stack.seed.tenant_id)
                .await
                .expect("read user")
                .expect("seeded user")
                .id
        }
    };
    let mut settings = CallingSettings::defaults(stack.seed.tenant_id, id_of(executive).await);
    let mut listed = Vec::new();
    for assistant in assistants {
        listed.push(fcp_core::Assistant {
            user_id: id_of(assistant).await,
            opted_in: true,
        });
    }
    settings.executive = Some(fcp_core::Executive {
        enabled: true,
        filter,
        alert: fcp_core::ExecutiveAlert::Assistants,
        assistants: listed,
    });
    settings.validate().expect("valid settings");
    storage
        .put_calling_settings(settings)
        .await
        .expect("store settings");
}

/// Dial a feature code and hang up once it is taken.
/// A feature code is answered, told, and hung up on by the node: a bare
/// 200 left the phone on a call nobody ended (`docs/DEEP_TESTING.md` D8).
async fn dial_code(stack: &Stack, phone: &Phone, code: &str) {
    let call = phone.call(code).await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("{code} was not taken: {e}"), phone)
            .await;
    }
    if !call.ended(RING_OUT).await {
        stack
            .fail(&format!("the node never ended the {code} call"), phone)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_executives_call_rings_the_assistant_who_pushes_it_on() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    make_executive(&stack, &bob, &[&carol], fcp_core::ExecutiveFilter::All).await;

    // Alice calls Bob: Carol's phone rings, told whose call it is and
    // with a ring of its own; Bob's does not.
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack.fail("the assistant did not ring", &alice).await;
    };
    assert!(
        at_carol
            .header("Alert-Info")
            .is_some_and(|a| a.contains("info=executive")),
        "Alert-Info: {:?}",
        at_carol.header("Alert-Info")
    );
    let from = at_carol.header("From").unwrap_or_default();
    assert!(from.contains(" for "), "From: {from}");
    assert!(
        bob_phone
            .incoming(Duration::from_millis(500))
            .await
            .is_none(),
        "the executive's phone rang with the assistant taking the call"
    );
    let carol_call = at_carol.answer().await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("Alice did not reach Carol: {e}"), &alice)
            .await;
    }

    // *63: Carol pushes the call to Bob, whose phone rings though his
    // calls are Carol's to take; she is let go once he answers.
    dial_code(&stack, &carol_phone, "*63").await;
    let Some(at_bob) = bob_phone.incoming(SOON).await else {
        stack
            .fail("the pushed call did not ring the executive", &carol_phone)
            .await;
    };
    let bob_call = at_bob.answer().await.unwrap();
    assert!(
        carol_call.ended(SOON).await,
        "the assistant is let go after the push"
    );
    assert!(
        !call.ended(QUIET).await,
        "Alice stays on the call, with Bob"
    );
    bob_call.hangup().await.unwrap();
    assert!(call.ended(SOON).await);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_executive_and_the_assistant_each_turn_the_handling_off() {
    let stack = stack_or_skip!();
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    make_executive(&stack, &bob, &[&carol], fcp_core::ExecutiveFilter::All).await;

    // A call from Alice rings Bob directly when...
    let rings_bob = |why: &'static str| {
        let (alice, bob_phone, bob) = (&alice, &bob_phone, &bob);
        let stack = &stack;
        async move {
            let call = alice.call(&bob.extension).await.unwrap();
            let Some(at_bob) = bob_phone.incoming(SOON).await else {
                stack.fail(why, alice).await;
            };
            let answered = at_bob.answer().await.unwrap();
            call.answered(SOON).await.expect("answered");
            answered.hangup().await.unwrap();
        }
    };
    // ...the assistant opts out (*68)...
    dial_code(&stack, &carol_phone, "*68").await;
    rings_bob("an opted-out assistant still took the call").await;
    dial_code(&stack, &carol_phone, "*67").await;
    // ...the executive turns the handling off (*66)...
    dial_code(&stack, &bob_phone, "*66").await;
    rings_bob("the executive's handling was off and the call did not ring him").await;
    dial_code(&stack, &bob_phone, "*65").await;
    // ...and the filter takes external calls only: Alice is internal.
    make_executive(&stack, &bob, &[&carol], fcp_core::ExecutiveFilter::External).await;
    rings_bob("an internal call was taken by the assistant").await;
    // Back on: the assistant takes it again.
    make_executive(&stack, &bob, &[&carol], fcp_core::ExecutiveFilter::All).await;
    let call = alice.call(&bob.extension).await.unwrap();
    if carol_phone.incoming(SOON).await.is_none() {
        stack
            .fail("the assistant did not take the call once back on", &alice)
            .await;
    }
    drop(call);

    // A user who assists nobody cannot opt in.
    let code = alice.call("*67").await.unwrap();
    assert_eq!(code.failed(SOON).await, Some(404));
    stack.finish().await;
}

// ---------------------------------------------------------------------------
// Monitor, whisper and barge (docs/ENTERPRISE_CALLING.md §3.11)
// ---------------------------------------------------------------------------

/// Ask the node to put `supervisor` on `call_id` in `mode`, answering at
/// `phone` when it rings: the node's status and the supervisor's call.
async fn supervise(
    stack: &Stack,
    call_id: &str,
    mode: &str,
    supervisor: &str,
    phone: &Phone,
) -> (u16, Option<fcp_testkit::Call>) {
    let url = loopback(stack.http, &format!("/calls/{call_id}/{mode}"));
    let body = format!(r#"{{"supervisor_extension":"{supervisor}"}}"#);
    let asked = tokio::spawn(async move { fcp_testkit::smoke::http_post(&url, &body).await });
    let answered = match phone.incoming(SOON).await {
        Some(ringing) => Some(ringing.answer().await.expect("the supervisor answers")),
        None => None,
    };
    let (status, text) = asked.await.unwrap().expect("POST");
    assert!(status == 200 || answered.is_none(), "{status}: {text}");
    (status, answered)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_supervisor_monitors_whispers_to_and_barges_into_a_call() {
    let Some(stack) = Stack::start_with("[b2bua.media_anchoring]\nmode = \"Always\"\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;

    let call = alice.call(&bob.extension).await.unwrap();
    let Some(ringing) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice).await;
    };
    let bob_call = ringing.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    let calls = stack.calls().await;
    let call_id = calls[0]["call_id"].as_str().expect("call id").to_string();
    // One voice at a time: the testkit hears a tone that dominates, and
    // two summed are neither.
    call.media().play_tone(440.0, 20_000);

    // Monitor: Carol hears both, neither hears her.
    let (status, supervisor) =
        supervise(&stack, &call_id, "monitor", &carol.extension, &carol_phone).await;
    assert_eq!(status, 200);
    let Some(carol_call) = supervisor else {
        stack
            .fail("the supervisor's phone did not ring", &carol_phone)
            .await;
    };
    if !carol_call.media().heard(440, SOON).await {
        stack
            .fail("the supervisor does not hear the caller", &carol_phone)
            .await;
    }
    call.media().stop_tone();
    bob_call.media().play_tone(880.0, 20_000);
    assert!(
        carol_call.media().heard(880, SOON).await,
        "the supervisor hears the callee"
    );
    bob_call.media().stop_tone();
    carol_call.media().play_tone(1400.0, 20_000);
    assert!(
        !bob_call.media().heard(1400, QUIET).await,
        "nobody hears a monitor"
    );
    let (code, body) = fcp_testkit::smoke::http_get(&loopback(
        stack.http,
        &format!("/calls/{call_id}/supervisor"),
    ))
    .await
    .unwrap();
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("\"monitor\""), "{body}");

    // Whisper (to the callee): Bob hears Carol, Alice does not.
    let (code, body) = fcp_testkit::smoke::http_post(
        &loopback(stack.http, &format!("/calls/{call_id}/whisper")),
        &format!(r#"{{"supervisor_extension":"{}"}}"#, carol.extension),
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "{body}");
    assert!(
        bob_call.media().heard(1400, SOON).await,
        "the callee hears the whisper"
    );
    assert!(
        !call.media().heard(1400, QUIET).await,
        "the caller never hears a whisper"
    );

    // Barge: Alice hears Carol too.
    let (code, body) = fcp_testkit::smoke::http_post(
        &loopback(stack.http, &format!("/calls/{call_id}/barge")),
        &format!(r#"{{"supervisor_extension":"{}"}}"#, carol.extension),
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "{body}");
    assert!(
        call.media().heard(1400, SOON).await,
        "the caller hears a barge"
    );

    // A second supervisor is refused; the first leaves.
    let (code, _) = fcp_testkit::smoke::http_post(
        &loopback(stack.http, &format!("/calls/{call_id}/monitor")),
        r#"{"supervisor_extension":"1001"}"#,
    )
    .await
    .unwrap();
    assert_eq!(code, 409, "one supervisor at a time");
    let (code, body) = fcp_testkit::smoke::http_post(
        &loopback(stack.http, &format!("/calls/{call_id}/supervisor/leave")),
        "{}",
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "{body}");
    assert!(carol_call.ended(SOON).await, "the supervisor is let go");
    assert!(!call.ended(QUIET).await, "the call goes on");
    let (code, _) = fcp_testkit::smoke::http_get(&loopback(
        stack.http,
        &format!("/calls/{call_id}/supervisor"),
    ))
    .await
    .unwrap();
    assert_eq!(code, 404);

    // Nobody can supervise a call that is not there.
    let (code, _) = fcp_testkit::smoke::http_post(
        &loopback(
            stack.http,
            "/calls/call-00000000-0000-0000-0000-000000000000/monitor",
        ),
        &format!(r#"{{"supervisor_extension":"{}"}}"#, carol.extension),
    )
    .await
    .unwrap();
    assert_eq!(code, 404);
    bob_call.hangup().await.unwrap();
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_on_a_shared_line_barges_into_its_active_appearance() {
    let Some(stack) = Stack::start_with("[b2bua.media_anchoring]\nmode = \"Always\"\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    shared_line(&stack, &alice, &["desk-a", "desk-b"], 2).await;
    let a = line_phone(&stack, "desk-a").await;
    let b = line_phone(&stack, "desk-b").await;
    let carol_phone = stack.phone(&carol).await;

    let call = carol_phone.call(LINE).await.unwrap();
    let Some(at_a) = a.incoming(SOON).await else {
        stack.fail("the line did not ring", &carol_phone).await;
    };
    let a_call = at_a.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");

    // Barge-in off: the active appearance is refused.
    let refused = b
        .call_with_headers(LINE, &[("Call-Info", &appearance(1))])
        .await
        .unwrap();
    assert_eq!(refused.failed(SOON).await, Some(403));

    // On: the second phone joins, heard by both.
    {
        use fcp_core::storage::{LineBindingStore, PhoneNumberStore};
        let storage = stack.seed.storage();
        let number = storage
            .get_phone_number_by_e164(LINE)
            .await
            .unwrap()
            .unwrap();
        let mut binding = storage
            .get_line_binding_by_phone_number(&number.id)
            .await
            .unwrap()
            .unwrap();
        binding.barge_in = true;
        storage.put_line_binding(binding).await.unwrap();
    }
    call.media().play_tone(440.0, 20_000);
    let barge = b
        .call_with_headers(LINE, &[("Call-Info", &appearance(1))])
        .await
        .unwrap();
    if let Err(e) = barge.answered(SOON).await {
        stack
            .fail(&format!("the barge-in was not answered: {e}"), &b)
            .await;
    }
    assert!(
        barge.media().heard(440, SOON).await,
        "the barging phone hears the caller"
    );
    call.media().stop_tone();
    barge.media().play_tone(1400.0, 20_000);
    assert!(
        call.media().heard(1400, SOON).await,
        "the caller hears the barging phone"
    );
    if !a_call.media().heard(1400, SOON).await {
        stack
            .fail("the line's phone does not hear the barge", &a)
            .await;
    }

    // The barging phone hangs up; the call goes on.
    barge.hangup().await.unwrap();
    assert!(
        !call.ended(QUIET).await,
        "the call goes on without the barging phone"
    );
    a_call.hangup().await.unwrap();
    assert!(call.ended(SOON).await);
    stack.finish().await;
}

// ---------------------------------------------------------------------------
// Live call control (docs/DEVELOPER_SURFACE.md phase 8)
// ---------------------------------------------------------------------------

/// Ask the node to `action` the call: its status and answer.
async fn control(stack: &Stack, call_id: &str, action: &str, body: &str) -> (u16, String) {
    let url = loopback(stack.http, &format!("/calls/{call_id}/{action}"));
    fcp_testkit::smoke::http_post(&url, body)
        .await
        .expect("POST")
}

/// The only call on the node: its id.
async fn only_call_id(stack: &Stack) -> String {
    let calls = stack.calls().await;
    calls[0]["call_id"].as_str().expect("call id").to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_api_holds_plays_into_and_hangs_up_a_call() {
    let Some(stack) = Stack::start_with("[b2bua.media_anchoring]\nmode = \"Always\"\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    let (alice_call, bob_call) = answered(&stack, &alice_phone, &bob, &bob_phone).await;
    let call_id = only_call_id(&stack).await;

    // Nothing for a call this node does not hold, or a party it has not.
    let (status, _) = control(
        &stack,
        "call-00000000-0000-0000-0000-000000000000",
        "hold",
        "{}",
    )
    .await;
    assert_eq!(status, 404);
    let (status, text) = control(&stack, &call_id, "hold", r#"{"party":"agent"}"#).await;
    assert_eq!(status, 400, "{text}");

    // Hold the caller: Alice hears the music and not Bob.
    let (status, text) = control(&stack, &call_id, "hold", r#"{"party":"caller"}"#).await;
    assert_eq!(status, 200, "{text}");
    bob_call.media().play_tone(1400.0, 20_000);
    if !alice_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await {
        stack
            .fail("Alice, held by the API, hears no music", &alice_phone)
            .await;
    }
    assert!(
        !alice_call
            .media()
            .heard(1400, Duration::from_millis(1500))
            .await,
        "Alice hears Bob while held"
    );
    held_as(&stack, Some("a")).await;
    // One party on hold at a time.
    let (status, _) = control(&stack, &call_id, "hold", r#"{"party":"callee"}"#).await;
    assert_eq!(status, 409);

    // Resume: she hears him again.
    let (status, text) = control(&stack, &call_id, "resume", "{}").await;
    assert_eq!(status, 200, "{text}");
    assert!(
        alice_call.media().heard(1400, SOON).await,
        "Alice does not hear Bob after the resume"
    );

    // Play the tenant's prompt to the callee.
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("chime.wav");
    tone_wav(&wav, 880.0, 3.0);
    let prompt = fcp_core::Prompt::new_file(
        stack.seed.tenant_id,
        "chime",
        fcp_core::PromptCategory::Custom,
        "en-US",
        wav,
    );
    let prompts =
        fcp_storage_postgres::prompt::PostgresPromptStore::new(stack.seed.storage().pool().clone());
    fcp_core::PromptStore::put_prompt(&prompts, prompt.clone())
        .await
        .unwrap();
    let (status, text) = control(
        &stack,
        &call_id,
        "play",
        &format!(r#"{{"prompt_id":"{}","to":"callee"}}"#, prompt.id),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("playback_id"), "{text}");
    assert!(
        bob_call.media().heard(880, SOON).await,
        "Bob does not hear the prompt"
    );
    let (status, _) = control(
        &stack,
        &call_id,
        "play",
        &format!(r#"{{"prompt_id":"{}"}}"#, fcp_core::PromptId::new()),
    )
    .await;
    assert_eq!(status, 404, "a prompt that is not the tenant's");

    // Say needs a text-to-speech engine, which this node has not.
    let (status, text) = control(&stack, &call_id, "say", r#"{"text":"Hello"}"#).await;
    assert_eq!(status, 503, "{text}");

    // Hang up: both phones are told.
    let (status, text) = control(&stack, &call_id, "hangup", "{}").await;
    assert_eq!(status, 200, "{text}");
    assert!(alice_call.ended(SOON).await, "Alice's call did not end");
    assert!(bob_call.ended(SOON).await, "Bob's call did not end");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_api_transfers_a_call_and_redirects_it_into_a_flow() {
    let Some(stack) = Stack::start_with("[b2bua.media_anchoring]\nmode = \"Always\"\n").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    let call_id = only_call_id(&stack).await;

    // Transfer the caller to Carol: she rings, answers, Bob is dropped.
    let (status, text) = control(
        &stack,
        &call_id,
        "transfer",
        &format!(r#"{{"party":"caller","target":"{}"}}"#, carol.extension),
    )
    .await;
    assert_eq!(status, 202, "{text}");
    let Some(ringing) = carol_phone.incoming(SOON).await else {
        stack.fail("Carol's phone did not ring", &alice).await;
    };
    let carol_call = ringing.answer().await.unwrap();
    assert!(bob_call.ended(SOON).await, "Bob was not dropped");
    carol_call.media().play_tone(440.0, 20_000);
    if !alice_call.media().heard(440, SOON).await {
        stack.fail("Alice does not hear Carol", &alice).await;
    }
    let (status, _) = control(
        &stack,
        &call_id,
        "transfer",
        r#"{"target":"sip:evil@elsewhere"}"#,
    )
    .await;
    assert_eq!(status, 400, "a target is dialed at this node");

    // Redirect the caller into a flow: Carol is dropped, and the flow runs
    // for Alice until it hangs up.
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "goodbye", "name": "Goodbye", "version": 1, "initial_node": "answer",
        "enabled": true, "tenant_id": stack.seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"type": "answer", "next": "wait"},
            "wait": {"type": "pause", "duration": 2, "next": "noted"},
            "noted": {"type": "log", "message": "redirected", "next": "bye"},
            "bye": {"type": "hangup"}
        }
    }))
    .unwrap();
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .unwrap();
    let (status, _) = control(&stack, &call_id, "redirect", r#"{"flow_id":"missing"}"#).await;
    assert_eq!(status, 404);
    let (status, text) = control(&stack, &call_id, "redirect", r#"{"flow_id":"goodbye"}"#).await;
    assert_eq!(status, 202, "{text}");
    assert!(carol_call.ended(SOON).await, "Carol was not dropped");
    if !alice_call.ended(Duration::from_secs(10)).await {
        stack.fail("the flow did not run for Alice", &alice).await;
    }
    let traces = fcp_storage_postgres::flow_trace::PostgresFlowTraceStore::new(
        stack.seed.storage().pool().clone(),
    );
    // The trace is written as the flow runs, after the call it ran on may
    // have ended: wait for its last node rather than read it once.
    let mut ran = Vec::new();
    for _ in 0..50 {
        ran = fcp_core::flow_trace::FlowTraceStore::query_flow_traces(
            &traces,
            &fcp_core::flow_trace::FlowTraceQuery {
                flow_id: Some("goodbye".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;
        if ran.iter().any(|t| t.node_id.as_deref() == Some("noted")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        ran.iter().any(|t| t.node_id.as_deref() == Some("noted")),
        "{ran:?}"
    );
    stack.finish().await;
}

/// With `auth_invites` on, the legs the node places to itself — a
/// transfer's, a forward's — carry no digest: they come from the node's own
/// address naming one of its calls, take that call's tenant and are let in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_nodes_own_legs_pass_invite_authentication() {
    let Some(stack) = Stack::start_configured("", "auth_invites = true").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let [(_, alice), (bob, bob_phone), (carol, carol_phone)] = three_phones(&stack).await;

    // A transfer: Alice sends Bob to Carol.
    let (alice_call, bob_call) = answered(&stack, &alice, &bob, &bob_phone).await;
    let refer_to = format!("sip:{}@{}", carol.extension, stack.sip);
    assert_eq!(alice_call.refer(&refer_to).await.unwrap(), 202);
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring for the transfer", &alice)
            .await;
    };
    let carol_call = at_carol.answer().await.unwrap();
    assert!(alice_call.ended(SOON).await, "Alice's leg was not dropped");
    carol_call.media().play_tone(660.0, 3000);
    assert!(
        bob_call.media().heard(660, SOON).await,
        "Bob does not hear Carol"
    );
    bob_call.hangup().await.unwrap();
    assert!(carol_call.ended(SOON).await);

    // A forward: Bob's calls go to Carol.
    stack
        .calling(&bob, |s| {
            s.forward_all = Some(CallTarget::Extension(carol.extension.clone()))
        })
        .await;
    let call = alice.call(&bob.extension).await.unwrap();
    let Some(at_carol) = carol_phone.incoming(SOON).await else {
        stack
            .fail("Carol's phone did not ring for the forward", &alice)
            .await;
    };
    let carol_call = at_carol.answer().await.unwrap();
    call.answered(SOON)
        .await
        .expect("the forwarded call is answered");
    call.hangup().await.unwrap();
    assert!(carol_call.ended(SOON).await);
    stack.finish().await;
}
