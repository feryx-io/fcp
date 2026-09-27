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
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// Long enough for a ring time to run out and the forward to land.
const RING_OUT: Duration = Duration::from_secs(15);
/// Long enough to be sure a phone that should not ring did not.
const QUIET: Duration = Duration::from_secs(2);

struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    /// Where the voicemail server would be: a testkit phone answers here.
    voicemail: SocketAddr,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database to test against.
    async fn start() -> Option<Self> {
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
    // The caller hangs up. The leg a flow dialed is not ended with the
    // caller's today (the emergency test tolerates the same); phase 2's
    // work in the B2BUA is where that belongs.
    call.hangup().await.unwrap();
    let _ = answered.ended(SOON).await;

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
