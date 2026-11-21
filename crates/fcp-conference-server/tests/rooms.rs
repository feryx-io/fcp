//! Rooms on the conference server over real SIP and RTP
//! (`docs/TEST_FLOOR.md`, T3): phones join a room directly and through
//! the call manager, hear each other through the mixer, enter a PIN,
//! mute and are muted, and leave. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{CallDisposition, CdrKind, HangupParty};
use fcp_core::TenantId;
use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::smoke::{http_get, http_post, http_request, loopback};
use fcp_testkit::{bins, free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-conference");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// A video room's `[conference]`: the raw test codec, and the fixed cost
/// table rather than a benchmark of a busy runner (a slow measurement once
/// left a webinar no budget for its first join).
const VIDEO: &str = "video_native_codecs_only = false\nvideo_benchmark_seconds = 0";

const ROOM: &str = "8000";

/// A running conference server on a fresh database.
struct Stack {
    conference: tokio::sync::Mutex<Option<Service>>,
    call_manager: tokio::sync::Mutex<Option<Service>>,
    /// The call manager's HTTP port, once it is started.
    call_manager_http: std::sync::Mutex<Option<u16>>,
    db: Option<TestDatabase>,
    seed: Seed,
    /// The conference server's SIP address: where a phone dials a room.
    sip: SocketAddr,
    http: u16,
    /// The first of the node's RTP ports.
    rtp_start: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// A conference server that believes nothing any peer claims about a
    /// call (`docs/EDGE_SECURITY.md` §2.3) — every phone here shares one
    /// loopback address, so trusting it would trust them all.
    async fn start() -> Option<Self> {
        Self::start_trusting("").await
    }

    /// As [`Stack::start`], believing `edges` — what a deployment does
    /// for the call manager and the gateway beside it, which name the
    /// record a leg belongs to.
    async fn start_trusting(edges: &str) -> Option<Self> {
        Self::start_with(edges, 60).await
    }

    /// As [`Stack::start_trusting`], with `pairs` RTP port pairs.
    async fn start_with(edges: &str, pairs: u16) -> Option<Self> {
        Self::start_conference(edges, pairs, "").await
    }

    /// As [`Stack::start_with`], with `conference` added to the
    /// `[conference]` section.
    async fn start_conference(edges: &str, pairs: u16, conference: &str) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_rtp_range(120);
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "e2e-conference"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip_port}"
{trusted_edges}

[storage]
database_url = "{db}"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[conference]
recordings_dir = "{dir}/recordings"
announcements_dir = "{dir}/announcements"
pin_timeout_secs = 5
{conference}

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
            rtp_end = rtp_start + 2 * pairs,
            trusted_edges = edges,
            dir = dir.path().display(),
        );
        let service = Service::start("conference", BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            conference: tokio::sync::Mutex::new(Some(service)),
            call_manager: tokio::sync::Mutex::new(None),
            call_manager_http: std::sync::Mutex::new(None),
            db: Some(db),
            seed,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            rtp_start,
            _dir: dir,
        })
    }

    /// Start a call manager on the same database, routing room numbers to
    /// this conference server; returns its SIP address.
    async fn start_call_manager(&self) -> SocketAddr {
        self.start_call_manager_with("").await
    }

    /// As [`Stack::start_call_manager`], with `extra` added to its
    /// configuration.
    async fn start_call_manager_with(&self, extra: &str) -> SocketAddr {
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_rtp_range(120);
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

[b2bua]
conference_server_uri = "sip:conference@{conf}"

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
            db = self.db.as_ref().expect("database").url(),
            conf = self.sip,
            rtp_end = rtp_start + 120,
            dir = self._dir.path().display(),
        );
        let bin = bins::path("fcp-call-manager", "fcp-call-manager");
        let service =
            Service::start("call-manager", bin, &config, &loopback(http, "/health")).await;
        *self.call_manager.lock().await = Some(service);
        *self.call_manager_http.lock().unwrap() = Some(http);
        SocketAddr::from(([127, 0, 0, 1], sip_port))
    }

    /// A phone that dials the conference server directly; no registration
    /// is needed, the room number is the Request-URI.
    async fn phone(&self, name: &str) -> Phone {
        Phone::new(name, "x", REALM, self.sip)
            .await
            .expect("phone binds")
    }

    /// What the call manager keeps for calls (`/status` `held`, #344).
    async fn call_manager_held(&self) -> serde_json::Value {
        let http = self
            .call_manager_http
            .lock()
            .unwrap()
            .expect("call manager");
        let (_, body) = http_get(&loopback(http, "/status")).await.expect("status");
        serde_json::from_str::<serde_json::Value>(&body).expect("json")["held"].clone()
    }

    async fn get(&self, path: &str) -> (u16, serde_json::Value) {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or_else(|_| serde_json::Value::String(body.clone()));
        (code, json)
    }

    async fn post(&self, path: &str) -> (u16, serde_json::Value) {
        let (code, body) = http_post(&loopback(self.http, path), "{}")
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or_else(|_| serde_json::Value::String(body.clone()));
        (code, json)
    }

    /// `method` with a JSON body (`{}` when none) on the node's HTTP API.
    async fn send(&self, method: &str, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
        let (code, body) = http_request(
            method,
            &loopback(self.http, path),
            &[],
            Some(body.unwrap_or("{}")),
        )
        .await
        .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
        let json: serde_json::Value =
            serde_json::from_str(&body).unwrap_or_else(|_| serde_json::Value::String(body.clone()));
        (code, json)
    }

    /// The room's participant ids as the API lists them, or none when
    /// the room is not (or no longer) there.
    async fn participants(&self) -> Vec<String> {
        let (code, room) = self.get(&format!("/conferences/{ROOM}")).await;
        if code != 200 {
            return Vec::new();
        }
        room["participants"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One participant's detail: `state` (`Active`, `Muted`), `is_host`,
    /// `display_name`, …
    async fn participant(&self, pid: &str) -> serde_json::Value {
        let (code, p) = self
            .get(&format!("/conferences/{ROOM}/participants/{pid}"))
            .await;
        assert_eq!(code, 200, "participant {pid}: {p}");
        p
    }

    /// The id of the participant whose caller id or display name contains
    /// `name` (a phone with no display name is known by its user part).
    async fn participant_named(&self, name: &str) -> String {
        // The room lists a participant a moment before the session detail
        // (caller id, display name) is there; look again until it is.
        let deadline = tokio::time::Instant::now() + SOON;
        let mut seen = Vec::new();
        loop {
            seen.clear();
            for pid in self.participants().await {
                let p = self.participant(&pid).await;
                let caller = p["caller_id"].as_str().unwrap_or("");
                let display = p["display_name"].as_str().unwrap_or("");
                if caller.contains(name) || display.contains(name) {
                    return pid;
                }
                seen.push(p);
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("no participant named {name}: {seen:?}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait until `pid`'s detail satisfies `want`.
    async fn wait_participant(&self, pid: &str, want: impl Fn(&serde_json::Value) -> bool) {
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let p = self.participant(pid).await;
            if want(&p) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "participant {pid}: {p}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait until the room lists `n` participants.
    async fn wait_participants(&self, n: usize) -> Vec<String> {
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            let p = self.participants().await;
            if p.len() == n {
                return p;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("room has {} participants, wanted {n}: {p:?}", p.len());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn fail(&self, why: &str, phone: &Phone) -> ! {
        let mut seen = Vec::new();
        while let Some(ev) = phone.next_event(Duration::from_millis(50)).await {
            seen.push(ev);
        }
        let mut logs = String::new();
        if let Some(s) = self.call_manager.lock().await.take() {
            logs.push_str(&s.kill().await);
        }
        if let Some(s) = self.conference.lock().await.take() {
            logs.push_str(&s.kill().await);
        }
        panic!("{why}\nphone {} saw: {seen:#?}\n{logs}", phone.username);
    }

    /// Fail saying `why`, with what the services logged.
    async fn fail_with_logs(&self, why: &str) -> ! {
        let mut logs = String::new();
        if let Some(s) = self.call_manager.lock().await.take() {
            logs.push_str(&s.kill().await);
        }
        if let Some(s) = self.conference.lock().await.take() {
            logs.push_str(&s.kill().await);
        }
        panic!("{why}\n{logs}");
    }

    async fn finish(mut self) {
        if let Some(s) = self.call_manager.lock().await.take() {
            s.stop().await;
        }
        if let Some(s) = self.conference.lock().await.take() {
            s.stop().await;
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

/// Dial the room on the conference server and wait for the answer.
async fn join(stack: &Stack, phone: &Phone) -> fcp_testkit::Call {
    let call = phone.call(ROOM).await.expect("INVITE sent");
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("{} was not admitted: {e}", phone.username), phone)
            .await;
    }
    call
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_phones_in_a_room_hear_each_other_and_leave() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;

    let alice_call = join(&stack, &alice).await;
    let bob_call = join(&stack, &bob).await;
    stack.wait_participants(2).await;

    // Through the mixer, each hears the other's tone.
    alice_call.media().play_tone(440.0, 1500);
    assert!(bob_call.media().heard(440, SOON).await, "Bob hears Alice");
    bob_call.media().play_tone(880.0, 1500);
    assert!(alice_call.media().heard(880, SOON).await, "Alice hears Bob");

    // The room is listed, and empties as they leave.
    let (code, rooms) = stack.get("/conferences").await;
    assert_eq!(code, 200, "{rooms}");
    assert!(rooms.to_string().contains(ROOM), "{rooms}");
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.wait_participants(1).await;
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;

    // A record of each leg (docs/CDR_PIPELINE.md §5): the room, the
    // participant, who hung up, the media that crossed.
    let records = stack.seed.cdrs(Some(CdrKind::Conference), 2, SOON).await;
    assert_eq!(records.len(), 2, "{records:#?}");
    for record in &records {
        let data = record.custom_data.as_ref().expect("leg data");
        assert_eq!(data["room_number"], ROOM, "{record:#?}");
        assert_eq!(record.to_uri, format!("conference:{ROOM}"));
        assert_eq!(record.disposition, CallDisposition::Answered, "{record:#?}");
        assert_eq!(
            record.hangup_party,
            Some(HangupParty::Caller),
            "{record:#?}"
        );
        assert!(record.rtp_packets_received.unwrap_or(0) > 0, "{record:#?}");
        assert!(record.codec_a_leg.is_some(), "{record:#?}");
    }
    assert!(
        records.iter().any(|r| r.from_uri.contains("alice")),
        "{records:#?}"
    );
    assert!(
        records.iter().any(|r| r.from_uri.contains("bob")),
        "{records:#?}"
    );
    assert_ne!(
        records[0].call_id, records[1].call_id,
        "two legs, two calls"
    );

    stack.finish().await;
}

/// A participant holds and resumes by re-INVITE, in the room. The room
/// keeps its participants' dialogs itself, so the node took a re-INVITE in
/// one for a request in no dialog and answered 481 (`DEEP_TESTING.md`
/// D2c's refusal): hold, adding video and sharing a screen all failed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_participant_holds_and_resumes_by_re_invite() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;
    let alice_call = join(&stack, &alice).await;
    let bob_call = join(&stack, &bob).await;
    stack.wait_participants(2).await;

    assert_eq!(
        alice_call
            .reinvite("sendonly")
            .await
            .expect("the hold is answered"),
        200,
        "a re-INVITE in the room's dialog"
    );
    assert_eq!(
        alice_call
            .reinvite("sendrecv")
            .await
            .expect("the resume is answered"),
        200
    );
    // Still in the room, and heard.
    stack.wait_participants(2).await;
    alice_call.media().play_tone(440.0, 1500);
    assert!(bob_call.media().heard(440, SOON).await, "Bob hears Alice");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// A caller who hangs up the moment they are answered leaves nobody in
/// the room. The node answers before the participant is a session (the
/// welcome message plays in between); a BYE then found nothing, was
/// answered 481, and the participant joined afterwards and stayed
/// (`docs/DEEP_TESTING.md` D2c, #318).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_who_hangs_up_on_the_answer_leaves_nobody_behind() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;
    let alice_call = join(&stack, &alice).await;
    stack.wait_participants(1).await;

    let bob_call = join(&stack, &bob).await;
    let code = bob_call.hangup().await;
    if !matches!(code, Ok(200)) {
        stack
            .fail(&format!("Bob's hang-up on the answer: {code:?}"), &bob)
            .await;
    }
    // Long enough for a join that outlived its caller to land.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stack.wait_participants(1).await.len(), 1);

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
}

/// Two baresip phones, a softphone that is not ours, in one room
/// (`docs/DEEP_TESTING.md` D2): each hears the other's tone through the
/// mixer, and the room empties as they leave. Skipped without baresip
/// unless `FCP_BARESIP_TESTS=required`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_baresip_phones_in_a_room_hear_each_other() {
    use fcp_testkit::baresip::{Baresip, PhoneSetup};
    let Some(baresip) = Baresip::find() else {
        return;
    };
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let setup = |user: &str, hz: f32| PhoneSetup {
        register: false,
        codecs: vec!["PCMU".into()],
        ..PhoneSetup::new(user, "x", REALM, stack.sip, hz)
    };
    let alice = baresip.phone(setup("alice", 440.0)).await.expect("alice");
    let bob = baresip.phone(setup("bob", 880.0)).await.expect("bob");
    let room = format!("sip:{ROOM}@{}", stack.sip);
    for phone in [&alice, &bob] {
        phone.dial(&room).await.expect("dial");
        if let Err(e) = phone.event(&["CALL_ESTABLISHED"], SOON).await {
            panic!("{}: {e}\n{}", phone.user, phone.log());
        }
    }
    stack.wait_participants(2).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        alice.heard_tone(880, 1500),
        "Alice hears Bob\n{}",
        alice.log()
    );
    assert!(bob.heard_tone(440, 1500), "Bob hears Alice\n{}", bob.log());
    assert!(!alice.heard_tone(440, 1500), "Alice hears herself");

    for (phone, left) in [(&alice, 1), (&bob, 0)] {
        phone.hangup().await.expect("hangup");
        phone
            .event(&["CALL_CLOSED"], SOON)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", phone.user));
        stack.wait_participants(left).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pin_protected_room_admits_the_right_pin_and_hangs_up_on_the_wrong_one() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, Some("1234"), Some("9999")).await;

    /// Digits as RFC 2833, `#` to finish.
    async fn dial_pin(call: &fcp_testkit::Call, pin: &str) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        for d in pin.chars().chain(std::iter::once('#')) {
            call.media().send_dtmf(d);
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    // The right PIN admits a guest.
    let alice = stack.phone("alice").await;
    let alice_call = join(&stack, &alice).await;
    dial_pin(&alice_call, "1234").await;
    let p = stack.wait_participants(1).await;
    assert_eq!(stack.participant(&p[0]).await["is_host"], false);

    // The host PIN makes a moderator.
    let bob = stack.phone("bob").await;
    let bob_call = join(&stack, &bob).await;
    dial_pin(&bob_call, "9999").await;
    stack.wait_participants(2).await;
    let bob_id = stack.participant_named("bob").await;
    stack
        .wait_participant(&bob_id, |p| p["is_host"] == true)
        .await;

    // A wrong PIN, three times: hung up.
    let carol = stack.phone("carol").await;
    let carol_call = join(&stack, &carol).await;
    for _ in 0..3 {
        dial_pin(&carol_call, "0000").await;
    }
    assert!(
        carol
            .expect(Duration::from_secs(10), |e| matches!(
                e,
                PhoneEvent::Bye { .. }
            ))
            .await
            .is_some(),
        "Carol is hung up after three wrong PINs"
    );
    assert_eq!(stack.wait_participants(2).await.len(), 2);

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mute_by_dtmf_and_by_api_and_a_kick() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;
    let alice_call = join(&stack, &alice).await;
    let bob_call = join(&stack, &bob).await;
    stack.wait_participants(2).await;
    let bob_id = stack.participant_named("bob").await;

    // Bob mutes himself with *6: the room says so, and Alice no longer
    // hears him.
    bob_call.media().send_dtmf('*');
    tokio::time::sleep(Duration::from_millis(150)).await;
    bob_call.media().send_dtmf('6');
    stack
        .wait_participant(&bob_id, |p| p["state"] == "Muted")
        .await;
    bob_call.media().play_tone(660.0, 1500);
    assert!(
        !alice_call.media().heard(660, Duration::from_secs(2)).await,
        "muted Bob is not heard"
    );

    // Unmuted by the API, he is heard again; muted by the API, listed so.
    let (code, body) = stack
        .post(&format!("/conferences/{ROOM}/unmute/{bob_id}"))
        .await;
    assert_eq!(code, 200, "{body}");
    stack
        .wait_participant(&bob_id, |p| p["state"] == "Active")
        .await;
    bob_call.media().play_tone(770.0, 1500);
    assert!(
        alice_call.media().heard(770, SOON).await,
        "unmuted Bob is heard"
    );
    let (code, body) = stack
        .post(&format!("/conferences/{ROOM}/mute/{bob_id}"))
        .await;
    assert_eq!(code, 200, "{body}");
    stack
        .wait_participant(&bob_id, |p| p["state"] == "Muted")
        .await;

    // Kicked, Bob's phone gets BYE.
    let (code, body) = stack
        .post(&format!("/conferences/{ROOM}/kick/{bob_id}"))
        .await;
    assert_eq!(code, 200, "{body}");
    assert!(
        bob.expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Bob is hung up"
    );
    stack.wait_participants(1).await;

    // Bob's leg is on the record as ended by the platform.
    let records = stack.seed.cdrs(Some(CdrKind::Conference), 1, SOON).await;
    let kicked = records
        .iter()
        .find(|r| r.from_uri.contains("bob"))
        .unwrap_or_else(|| panic!("no record of Bob's leg: {records:#?}"));
    assert_eq!(
        kicked.hangup_party,
        Some(HangupParty::System),
        "{kicked:#?}"
    );
    assert_eq!(kicked.sip_reason.as_deref(), Some("Kicked"), "{kicked:#?}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

/// The next roster NOTIFY in subscription `call_id`: its state and body.
async fn roster(phone: &Phone, call_id: &str) -> (String, String) {
    match phone
        .expect(SOON, |e| {
            matches!(e, PhoneEvent::Notify { call_id: c, event, .. }
                if c == call_id && event == "conference")
        })
        .await
    {
        Some(PhoneEvent::Notify { state, body, .. }) => (state.unwrap_or_default(), body),
        _ => panic!("no roster NOTIFY in {call_id}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_roster_is_told_whole_then_by_what_changes_and_only_to_a_participant() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;
    let carol = stack.phone("carol").await;
    let alice_call = join(&stack, &alice).await;
    let bob_call = join(&stack, &bob).await;
    stack.wait_participants(2).await;
    let bob_id = stack.participant_named("bob").await;

    // Someone not in the room is refused, as is one naming a call that
    // is not theirs.
    let stranger = carol
        .subscribe(ROOM, "conference", 600, None)
        .await
        .unwrap();
    assert_eq!(stranger.code, 403, "a stranger");
    let forged = carol
        .subscribe_with(
            ROOM,
            "conference",
            600,
            None,
            vec![(
                "Target-Dialog".into(),
                format!("{};local-tag=x;remote-tag=y", alice_call.call_id()),
            )],
        )
        .await
        .unwrap();
    assert_eq!(forged.code, 403, "a call named with the wrong tags");

    // Alice, naming her call: everyone, by name, version 1.
    let target = alice_call.target_dialog().await.unwrap();
    let mut sub = alice
        .subscribe_with(
            ROOM,
            "conference",
            600,
            None,
            vec![("Target-Dialog".into(), target)],
        )
        .await
        .unwrap();
    assert_eq!(sub.code, 200);
    let (state, first) = roster(&alice, &sub.call_id).await;
    assert!(state.starts_with("active;expires="), "{state}");
    assert!(first.contains("state=\"full\" version=\"1\""), "{first}");
    assert!(first.contains("<user-count>2</user-count>"), "{first}");
    assert!(
        first.contains("<display-text>alice</display-text>"),
        "{first}"
    );
    assert!(
        first.contains("<display-text>bob</display-text>"),
        "{first}"
    );

    // Bob mutes himself: version 2 tells of Bob alone, heard no more.
    bob_call.media().send_dtmf('*');
    tokio::time::sleep(Duration::from_millis(150)).await;
    bob_call.media().send_dtmf('6');
    let (_, muted) = roster(&alice, &sub.call_id).await;
    assert!(muted.contains("state=\"partial\" version=\"2\""), "{muted}");
    assert!(muted.contains(&format!("sip:{bob_id}@")), "{muted}");
    assert!(
        muted.contains("<type>audio</type><status>recvonly</status>"),
        "{muted}"
    );
    assert!(
        !muted.contains("<display-text>alice</display-text>"),
        "{muted}"
    );

    // Bob leaves: version 3 deletes him.
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    let (_, left) = roster(&alice, &sub.call_id).await;
    assert!(left.contains("version=\"3\""), "{left}");
    assert!(left.contains(&format!("sip:{bob_id}@")), "{left}");
    assert!(left.contains("state=\"deleted\"/>"), "{left}");
    assert!(left.contains("<user-count>1</user-count>"), "{left}");

    // Alice ends it.
    assert_eq!(alice.resubscribe(&mut sub, 0, None).await.unwrap(), 200);
    let (state, _) = roster(&alice, &sub.call_id).await;
    assert!(state.starts_with("terminated"), "{state}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

/// A phone through the call manager (`docs/SOFTPHONE_VIDEO.md` §2.6, F2b)
/// joins by a host link, without a PIN, and asks for the floor's notices;
/// it follows the roster in its own call, the call manager carrying the
/// SUBSCRIBE into the room's call and the NOTIFYs back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_through_the_call_manager_joins_by_link_and_follows_the_roster() {
    let Some(stack) = Stack::start_conference(
        r#"trusted_edges = ["127.0.0.1"]"#,
        60,
        "join_secret = \"join-s3cret\"",
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack.seed.meeting(ROOM, None, None).await;
    let cm = stack.start_call_manager().await;
    let alice_user = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.phone("bob").await;
    let bob_call = join(&stack, &bob).await;
    let alice = Phone::new(&alice_user.username, &alice_user.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);

    let token = fcp_core::JoinToken::new(
        stack.seed.tenant_id,
        ROOM,
        "Alice Host",
        fcp_core::JoinRole::Host,
        Duration::from_secs(600),
    )
    .mint(b"join-s3cret");
    let alice_call = alice
        .call_with_headers(
            ROOM,
            &[("X-FCP-Join", &token), ("Recv-Info", "fcp-content")],
        )
        .await
        .unwrap();
    if let Err(e) = alice_call.answered(Duration::from_secs(10)).await {
        stack
            .fail(&format!("Alice's link did not admit her: {e}"), &alice)
            .await;
    }
    stack.wait_participants(2).await;
    let alice_id = stack.participant_named("Alice Host").await;
    let bob_id = stack.participant_named("bob").await;
    let seen = stack.participant(&alice_id).await;
    assert_eq!(seen["is_host"], true, "the link's role: {seen}");
    assert_eq!(seen["floor_notices"], true, "{seen}");
    assert_eq!(stack.participant(&bob_id).await["floor_notices"], false);

    // The roster, in Alice's call.
    assert_eq!(
        alice_call
            .subscribe_in_call("conference", 600)
            .await
            .unwrap(),
        200
    );
    let (state, first) = roster(&alice, alice_call.call_id()).await;
    assert!(state.starts_with("active;expires="), "{state}");
    assert!(first.contains("state=\"full\" version=\"1\""), "{first}");
    assert!(first.contains("<user-count>2</user-count>"), "{first}");
    assert!(
        first.contains("<display-text>Alice Host</display-text>"),
        "{first}"
    );
    assert!(
        first.contains("<roles><entry>host</entry></roles>"),
        "{first}"
    );

    // Bob mutes himself: version 2 tells of Bob alone.
    bob_call.media().send_dtmf('*');
    tokio::time::sleep(Duration::from_millis(150)).await;
    bob_call.media().send_dtmf('6');
    let (_, muted) = roster(&alice, alice_call.call_id()).await;
    assert!(muted.contains("state=\"partial\" version=\"2\""), "{muted}");
    assert!(muted.contains(&format!("sip:{bob_id}@")), "{muted}");
    assert!(
        muted.contains("<type>audio</type><status>recvonly</status>"),
        "{muted}"
    );

    // Alice ends it.
    assert_eq!(
        alice_call.subscribe_in_call("conference", 0).await.unwrap(),
        200
    );
    let (state, _) = roster(&alice, alice_call.call_id()).await;
    assert!(state.starts_with("terminated"), "{state}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// A join token is for its own room: dialed at another, it is refused
/// rather than admitting the caller somewhere nobody placed them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_join_token_dialed_at_another_room_is_refused() {
    let Some(stack) = Stack::start_conference("", 60, "join_secret = \"join-s3cret\"").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack.seed.meeting(ROOM, None, None).await;
    stack.seed.meeting("8001", None, None).await;
    let token = fcp_core::JoinToken::new(
        stack.seed.tenant_id,
        "8001",
        "Mallory",
        fcp_core::JoinRole::Host,
        Duration::from_secs(600),
    )
    .mint(b"join-s3cret");
    let mallory = stack.phone("mallory").await;
    let call = mallory
        .call_with_headers(ROOM, &[("X-FCP-Join", &token)])
        .await
        .unwrap();
    assert_eq!(call.failed(SOON).await, Some(403));
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_does_not_choose_the_name_the_room_sees() {
    // `docs/EDGE_SECURITY.md` §2.3. The room took its participant's name
    // and number from `P-Asserted-Identity` — "more trusted on SBC/trunk
    // calls", except nothing checked that the peer was an SBC or a trunk.
    // Anyone who could dial the room could sit in it under any name, and
    // name the record their leg was filed under besides.
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let mallory = stack.phone("mallory").await;

    let call = mallory
        .call_with_headers(
            ROOM,
            &[
                ("P-Asserted-Identity", "<sip:+15125550100@carrier.example>"),
                ("X-FCP-Call-Id", "call-11111111-1111-1111-1111-111111111111"),
            ],
        )
        .await
        .expect("INVITE sent");
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("mallory was not admitted: {e}"), &mallory)
            .await;
    }
    stack.wait_participants(1).await;
    // The room knows them as the `From` they actually dialled from —
    // this panics if it knows them as the identity they asserted.
    let pid = stack.participant_named("mallory").await;
    let seen = stack.participant(&pid).await;
    let caller = seen["caller_id"].as_str().unwrap_or("");
    assert!(
        !caller.contains("15125550100"),
        "the room shows a number the caller asserted: {seen}"
    );

    assert_eq!(call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;

    // And the leg's record is the room's own, not one the caller named.
    let legs = stack.seed.cdrs(Some(CdrKind::Conference), 1, SOON).await;
    assert_ne!(
        legs[0].call_id.to_string(),
        "call-11111111-1111-1111-1111-111111111111",
        "the caller filed its leg against a record it named: {:#?}",
        legs[0]
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_room_is_reached_through_the_call_manager() {
    // The call manager beside it names the record each leg belongs to in
    // `X-FCP-Call-Id`, which is believed because it is listed as a
    // trusted edge (`docs/EDGE_SECURITY.md` §2.3).
    let Some(stack) = Stack::start_trusting(r#"trusted_edges = ["127.0.0.1"]"#).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack.seed.meeting(ROOM, None, None).await;
    let cm = stack.start_call_manager().await;
    let alice_user = stack.seed.user("alice", "alice-pw", "1001").await;

    // Bob dials the conference server; Alice, registered at the call
    // manager, dials the room number as she would an extension.
    let bob = stack.phone("bob").await;
    let bob_call = join(&stack, &bob).await;
    let alice = Phone::new(&alice_user.username, &alice_user.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);
    let alice_call = alice.call(ROOM).await.unwrap();
    if let Err(e) = alice_call.answered(Duration::from_secs(10)).await {
        stack
            .fail(
                &format!("Alice was not put through to the room: {e}"),
                &alice,
            )
            .await;
    }
    stack.wait_participants(2).await;

    alice_call.media().play_tone(440.0, 1500);
    assert!(
        bob_call.media().heard(440, SOON).await,
        "Bob hears Alice through the call manager"
    );
    bob_call.media().play_tone(880.0, 1500);
    assert!(alice_call.media().heard(880, SOON).await, "Alice hears Bob");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.wait_participants(1).await;
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;

    // Alice's leg in the room carries the call manager's call id, so it
    // sits beside that call's record (docs/CDR_PIPELINE.md §5); Bob, who
    // dialed the room directly, has a call id of his own.
    let calls = stack.seed.cdrs(Some(CdrKind::Call), 1, SOON).await;
    let legs = stack.seed.cdrs(Some(CdrKind::Conference), 2, SOON).await;
    let alice_call_record = calls
        .iter()
        .find(|r| r.from_uri.contains("alice"))
        .unwrap_or_else(|| panic!("no call record for Alice: {calls:#?}"));
    let alice_leg = legs
        .iter()
        .find(|r| r.call_id == alice_call_record.call_id)
        .unwrap_or_else(|| panic!("no leg with Alice's call id: {legs:#?}"));
    assert_eq!(alice_leg.kind, CdrKind::Conference);
    let bob_leg = legs
        .iter()
        .find(|r| r.from_uri.contains("bob"))
        .unwrap_or_else(|| panic!("no leg for Bob: {legs:#?}"));
    assert_ne!(bob_leg.call_id, alice_call_record.call_id);
    stack.finish().await;
}

/// Two tenants that both number a meeting 8000 meet in two rooms
/// (`docs/DEEP_TESTING.md` D6). The call manager names each caller's
/// tenant to the conference server, which believes it from a trusted edge
/// and keeps a room per tenant and number: a caller in one hears nothing
/// of the other, the node lists both rooms with their tenants, and a
/// tenant's request about its room sees its own callers only. Before,
/// the second tenant's callers were refused (403, then 500) and the
/// node's rooms were keyed by number alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_tenants_with_one_room_number_meet_apart() {
    let Some(stack) = Stack::start_trusting(r#"trusted_edges = ["127.0.0.1"]"#).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let tenant_a = stack.seed.tenant_id;
    let tenant_b = stack.seed.tenant("other", "other.e2e.local").await;
    stack.seed.meeting(ROOM, None, None).await;
    stack.seed.meeting_in(tenant_b, ROOM, None, None).await;
    let alice_user = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob_user = stack.seed.user("bob", "bob-pw", "1002").await;
    stack.seed.user_in(tenant_b, "bea", "bea-pw", "2001").await;
    // The second tenant's caller is placed by an edge, as a gateway would:
    // the call manager's single realm registers only the first tenant's.
    let cm = stack
        .start_call_manager_with("[e911]\ntrusted_edges = [\"127.0.0.0/8\"]")
        .await;

    let alice = Phone::new(&alice_user.username, &alice_user.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);
    let alice_call = alice.call(ROOM).await.unwrap();
    if let Err(e) = alice_call.answered(SOON).await {
        stack
            .fail(
                &format!("Alice was not put through to her room: {e}"),
                &alice,
            )
            .await;
    }
    let edge = Phone::new("edge", "x", REALM, cm).await.unwrap();
    let tenant_b_str = tenant_b.to_string();
    let bea_call = edge
        .call_with_headers(
            ROOM,
            &[
                (fcp_core::CALLER_TENANT_HEADER, tenant_b_str.as_str()),
                (fcp_core::CALLER_USER_HEADER, "bea"),
            ],
        )
        .await
        .unwrap();
    if let Err(e) = bea_call.answered(SOON).await {
        stack
            .fail(&format!("Bea was not put through to her room: {e}"), &edge)
            .await;
    }
    // A second caller of the first tenant. (Not dialling the node
    // directly: with two tenants an address names neither, and the node
    // refuses it.)
    let bob = Phone::new(&bob_user.username, &bob_user.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(bob.register().await.unwrap(), 200);
    let bob_call = bob.call(ROOM).await.unwrap();
    if let Err(e) = bob_call.answered(SOON).await {
        stack
            .fail(&format!("Bob was not put through to his room: {e}"), &bob)
            .await;
    }

    alice_call.media().play_tone(440.0, 3000);
    assert!(
        bob_call.media().heard(440, SOON).await,
        "Bob, in Alice's room, hears her"
    );
    assert!(
        !bea_call.media().heard(440, Duration::from_secs(2)).await,
        "Bea, in the other tenant's room of the same number, hears nothing of Alice"
    );
    bea_call.media().play_tone(880.0, 3000);
    assert!(
        !bob_call.media().heard(880, Duration::from_secs(2)).await,
        "nor does Bob hear Bea"
    );

    // The node holds two rooms numbered 8000, each with its tenant.
    let (code, listed) = stack.get("/conferences").await;
    assert_eq!(code, 200, "{listed}");
    let rooms = listed["conferences"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let of = |tenant: TenantId| {
        rooms
            .iter()
            .find(|r| r["room_number"] == ROOM && r["tenant_id"] == serde_json::json!(tenant))
            .map(|r| r["participants"].as_u64().unwrap_or(0))
    };
    assert_eq!(of(tenant_a), Some(2), "{listed}");
    assert_eq!(of(tenant_b), Some(1), "{listed}");

    // A tenant's request about its room sees its callers, not the
    // other's: the admin API names the tenant on every request it sends.
    let tenant_header = |t: TenantId| t.to_string();
    for (tenant, want) in [(tenant_a, 2), (tenant_b, 1)] {
        let (code, body) = http_request(
            "GET",
            &loopback(stack.http, &format!("/conferences/{ROOM}")),
            &[("x-fcp-tenant-id", tenant_header(tenant).as_str())],
            None,
        )
        .await
        .unwrap();
        assert_eq!(code, 200, "{body}");
        let room: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            room["participant_count"], want,
            "tenant {tenant}'s room: {room}"
        );
    }

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bea_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

/// Two callers who join a room at the same moment, talk, leave and come
/// back, round after round, hear each other every time
/// (`docs/DEEP_TESTING.md` D6). The meeting is not permanent, so the
/// room is removed when it empties and made again by the next caller.
/// Before, two callers making the room at once could each make it — the
/// second replacing the first's — and a caller joining while an emptied
/// room was being removed was left in a room nobody else reached: the
/// soak's two-caller room heard silence on two calls in three.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callers_meet_in_a_room_made_again_round_after_round() {
    let Some(stack) = Stack::start_trusting(r#"trusted_edges = ["127.0.0.1"]"#).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack.seed.meeting(ROOM, None, None).await;
    stack
        .seed
        .execute("UPDATE conference_meetings SET permanent = false")
        .await;
    let cm = stack.start_call_manager().await;
    let a = stack.seed.user("alice", "alice-pw", "1001").await;
    let b = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice = Phone::new(&a.username, &a.password, REALM, cm)
        .await
        .unwrap();
    let bob = Phone::new(&b.username, &b.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);
    assert_eq!(bob.register().await.unwrap(), 200);
    for round in 0..5 {
        let (alice_call, bob_call) = tokio::join!(alice.call(ROOM), bob.call(ROOM));
        let (alice_call, bob_call) = (alice_call.unwrap(), bob_call.unwrap());
        let (a_in, b_in) = tokio::join!(alice_call.answered(SOON), bob_call.answered(SOON));
        if let Err(e) = a_in.and(b_in) {
            stack
                .fail(&format!("round {round}: not both put through: {e}"), &alice)
                .await;
        }
        alice_call.media().play_tone(440.0, 3000);
        bob_call.media().play_tone(880.0, 3000);
        let (bob_hears, alice_hears) = tokio::join!(
            bob_call.media().heard(440, SOON),
            alice_call.media().heard(880, SOON)
        );
        assert!(
            bob_hears && alice_hears,
            "round {round}: Bob hears Alice {bob_hears}, Alice hears Bob {alice_hears}"
        );
        let _ = tokio::join!(alice_call.hangup(), bob_call.hangup());
        // Back at once, and a little later each round: into the room as
        // it is being removed, and after.
        tokio::time::sleep(Duration::from_millis(10 * round)).await;
    }
    stack.finish().await;
}

/// A caller is answered only on ports the node holds. The pool knows its
/// own pairs, not the host's: a pair another process held was put in the
/// answer, the 200 OK went out, and the bind after it failed — the caller
/// sat in a call no room listed (`docs/DEEP_TESTING.md` D6). All but four
/// of the node's fifty pairs are held here, so nearly every pick hits one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callers_are_answered_on_ports_the_node_could_bind() {
    let Some(stack) = Stack::start_with("", 50).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let start = stack.rtp_start;
    let _held: Vec<_> = (0..46)
        .map(|pair| start + 2 * pair)
        .flat_map(|port| [port, port + 1])
        .filter_map(|port| std::net::UdpSocket::bind(("0.0.0.0", port)).ok())
        .collect();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;

    let alice_call = join(&stack, &alice).await;
    let bob_call = join(&stack, &bob).await;
    stack.wait_participants(2).await;
    alice_call.media().play_tone(440.0, 1500);
    assert!(bob_call.media().heard(440, SOON).await, "Bob hears Alice");
    bob_call.media().play_tone(880.0, 1500);
    assert!(alice_call.media().heard(880, SOON).await, "Alice hears Bob");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// A node with no port pair left refuses a caller with 503 rather than
/// leave them ringing (found measuring `docs/WEBINARS.md` W2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_is_refused_when_no_port_pair_is_left() {
    let Some(stack) = Stack::start_with("", 50).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let start = stack.rtp_start;
    let _held: Vec<_> = (0..50)
        .map(|pair| start + 2 * pair)
        .flat_map(|port| [port, port + 1])
        .filter_map(|port| std::net::UdpSocket::bind(("0.0.0.0", port)).ok())
        .collect();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let call = alice.call(ROOM).await.expect("INVITE sent");
    let refused = call.answered(SOON).await.expect_err("no ports, no answer");
    assert!(refused.to_string().contains("503"), "{refused}");
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// Calls into a room through the call manager leave nothing behind there
/// once they end (#372): the soak's call manager kept about one more thing
/// for every room call (`fcp_node_held`). Callers leave both ways — the
/// caller hangs up, or the room is emptied and made again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn room_calls_through_the_call_manager_leave_nothing_behind() {
    const ROUNDS: usize = 12;
    let Some(stack) = Stack::start_trusting(r#"trusted_edges = ["127.0.0.1"]"#).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack.seed.meeting(ROOM, None, None).await;
    stack
        .seed
        .execute("UPDATE conference_meetings SET permanent = false")
        .await;
    // Both phones share one source, placing calls back to back.
    let cm = stack
        .start_call_manager_with("[security]\nmax_invite_cps_per_source = 0\n")
        .await;
    let a = stack.seed.user("alice", "alice-pw", "1001").await;
    let b = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice = Phone::new(&a.username, &a.password, REALM, cm)
        .await
        .unwrap();
    let bob = Phone::new(&b.username, &b.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(alice.register().await.unwrap(), 200);
    assert_eq!(bob.register().await.unwrap(), 200);
    for round in 0..ROUNDS {
        let (alice_call, bob_call) = tokio::join!(alice.call(ROOM), bob.call(ROOM));
        let (alice_call, bob_call) = (alice_call.unwrap(), bob_call.unwrap());
        let (a_in, b_in) = tokio::join!(alice_call.answered(SOON), bob_call.answered(SOON));
        if let Err(e) = a_in.and(b_in) {
            stack
                .fail(&format!("round {round}: not both put through: {e}"), &alice)
                .await;
        }
        stack.wait_participants(2).await;
        let _ = tokio::join!(alice_call.hangup(), bob_call.hangup());
        stack.wait_participants(0).await;
        // Within the conference server's ten INVITEs a second from one address.
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let deadline = tokio::time::Instant::now() + SOON;
    let held = loop {
        let held = stack.call_manager_held().await;
        let ended = held["dialogs"] == 0 && held["calls"] == 0 && held["flows"] == 0;
        if ended || tokio::time::Instant::now() > deadline {
            break held;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    eprintln!("held after {ROUNDS} rounds: {held}");
    assert_eq!(held["dialogs"], 0, "{held}");
    assert_eq!(held["calls"], 0, "{held}");
    assert_eq!(held["flows"], 0, "{held}");
    assert_eq!(held["media_sessions"], 0, "{held}");
    stack.finish().await;
}

/// Dial the room asking for each person's own stream
/// (`docs/FORWARDING_ROOMS.md` §2.3), with VP8 video.
async fn join_forwarding(stack: &Stack, phone: &Phone, ask: bool) -> fcp_testkit::Call {
    phone.enable_video(fcp_testkit::video::VideoMode::Plain);
    phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    let headers: &[(&str, &str)] = if ask {
        &[("Recv-Info", "fcp-content, fcp-view")]
    } else {
        &[]
    };
    let call = phone
        .call_with_headers(ROOM, headers)
        .await
        .expect("INVITE sent");
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("{} was not admitted: {e}", phone.username), phone)
            .await;
    }
    call
}

/// Send frames from `senders` until `done` holds of what `viewer` has
/// received, or fail after a while saying `what`.
async fn pump_until(
    stack: &Stack,
    senders: &[&fcp_testkit::Call],
    viewer: &fcp_testkit::Call,
    what: &str,
    done: impl Fn(&std::collections::HashMap<u32, u64>) -> bool,
) -> std::collections::HashMap<u32, u64> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let got = viewer
            .video()
            .expect("the viewer has video")
            .received_by_ssrc();
        if done(&got) {
            return got;
        }
        if tokio::time::Instant::now() >= deadline {
            stack
                .fail_with_logs(&format!("never: {what}; received {got:?}"))
                .await;
        }
        for s in senders {
            s.video()
                .expect("a sender has video")
                .send_frames(1, 10)
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(33)).await;
    }
}

/// The SSRC a roster says its subscriber receives `pid` on.
fn src_id_of(roster: &str, pid: &str) -> Option<u32> {
    let user = &roster[roster.find(&format!("sip:{pid}@"))?..];
    let user = &user[..user.find("</user>")?];
    let video = &user[user.find("<media id=\"video\">")?..];
    let video = &video[..video.find("</media>")?];
    let at = video.find("<src-id>")? + "<src-id>".len();
    video[at..at + video[at..].find('<')?].parse().ok()
}

/// A forwarding room over SIP (`docs/FORWARDING_ROOMS.md`, R3): a caller
/// that says `Recv-Info: fcp-view` in a meeting whose `video_mode` is `auto`
/// is told so in the answer and sent each person's own stream, under an
/// SSRC its roster names; its `fcp-view` INFO changes what it is sent. A
/// caller that did not ask gets the composite, and its INFO is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_that_asks_is_forwarded_each_persons_stream_and_told_whose_it_is() {
    let Some(stack) = Stack::start_conference("", 60, VIDEO).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({
                "video_enabled": true,
                "video_codecs": ["VP8"],
                "video_mode": "auto",
            }),
        )
        .await;
    let alice = stack.phone("alice").await;
    let carol = stack.phone("carol").await;
    let bob = stack.phone("bob").await;
    let alice_call = join_forwarding(&stack, &alice, true).await;
    let carol_call = join_forwarding(&stack, &carol, true).await;
    let bob_call = join_forwarding(&stack, &bob, true).await;
    stack.wait_participants(3).await;
    let alice_id = stack.participant_named("alice").await;
    let carol_id = stack.participant_named("carol").await;
    let bob_id = stack.participant_named("bob").await;
    assert_eq!(
        stack.participant(&bob_id).await["view_streams"],
        serde_json::json!(true),
        "the room forwards to Bob"
    );

    // The default view: the speaker, then everyone else — three streams.
    let got = pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "three streams for Bob",
        |got| got.len() >= 3 && got.values().all(|n| *n >= 3),
    )
    .await;

    // Bob's roster says whose each SSRC is.
    assert_eq!(
        bob_call.subscribe_in_call("conference", 600).await.unwrap(),
        200
    );
    let (_, full) = roster(&bob, bob_call.call_id()).await;
    let alice_ssrc = src_id_of(&full, &alice_id).expect("Alice's stream named");
    let carol_ssrc = src_id_of(&full, &carol_id).expect("Carol's stream named");
    assert!(got.contains_key(&alice_ssrc), "{full}\n{got:?}");
    assert!(got.contains_key(&carol_ssrc), "{full}\n{got:?}");
    assert!(full.contains("<media id=\"speaker\">"), "{full}");
    assert_eq!(src_id_of(&full, &bob_id), None, "not his own");

    // Bob asks for Carol alone: Alice's stream stops.
    let view = format!(r#"{{"streams":[{{"participant":"{carol_id}","max_height":360}}]}}"#);
    let info = |body: String| {
        let call = &bob_call;
        async move {
            call.send_in_call(
                sip_core::Method::Info,
                &[
                    ("Info-Package", fcp_core::VIEW_INFO_PACKAGE),
                    ("Content-Type", fcp_core::VIEW_INFO_TYPE),
                ],
                body.as_bytes(),
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(info(view).await, 200);
    assert_eq!(
        info("{\"streams\":[{}]}".into()).await,
        400,
        "a view that means nothing"
    );
    // Give the room a tick, then count afresh.
    pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "a tick",
        |_| true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = bob_call.video().unwrap().received_by_ssrc();
    let after = pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "Carol's stream going on",
        |got| {
            got.get(&carol_ssrc).copied().unwrap_or(0)
                >= before.get(&carol_ssrc).copied().unwrap_or(0) + 10
        },
    )
    .await;
    for (ssrc, n) in &after {
        if *ssrc != carol_ssrc {
            assert_eq!(
                Some(n),
                before.get(ssrc),
                "only Carol's stream goes on: {before:?} then {after:?}"
            );
        }
    }

    // A caller that did not ask is sent the composite and cannot send a view.
    let dave = stack.phone("dave").await;
    let dave_call = join_forwarding(&stack, &dave, false).await;
    stack.wait_participants(4).await;
    let dave_id = stack.participant_named("dave").await;
    assert_eq!(
        stack.participant(&dave_id).await["view_streams"],
        serde_json::json!(false)
    );
    assert_eq!(
        dave_call
            .send_in_call(
                sip_core::Method::Info,
                &[
                    ("Info-Package", fcp_core::VIEW_INFO_PACKAGE),
                    ("Content-Type", fcp_core::VIEW_INFO_TYPE),
                ],
                b"{}",
            )
            .await
            .unwrap(),
        469
    );

    for call in [&alice_call, &carol_call, &bob_call, &dave_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.finish().await;
}

/// Send one keyframe a frame from `sender` — on the simulcast layers
/// `rids` when it sends simulcast, else its one stream — until `done`
/// holds of `viewer`'s video, or fail after `within` saying `what`.
async fn pump_layers(
    stack: &Stack,
    sender: &fcp_testkit::Call,
    rids: Option<&[&str]>,
    viewer: &fcp_testkit::Call,
    what: &str,
    within: Duration,
    done: impl Fn(&fcp_testkit::video::VideoLeg) -> bool,
) {
    let deadline = tokio::time::Instant::now() + within;
    let leg = viewer.video().expect("the viewer has video");
    loop {
        if done(leg) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            let why = format!(
                "never: {what}; received {:?}, layers {:?}",
                leg.received_by_ssrc(),
                leg.layer_by_ssrc()
            );
            stack.fail_with_logs(&why).await;
        }
        let video = sender.video().expect("the sender has video");
        match rids {
            Some(rids) => video.send_layers(1, 1, rids).await.unwrap(),
            None => video.send_frames(1, 1).await.unwrap(),
        }
        tokio::time::sleep(Duration::from_millis(33)).await;
    }
}

/// Send `view` as `call`'s `fcp-view` INFO; the status it was answered.
async fn send_view(call: &fcp_testkit::Call, view: &str) -> u16 {
    call.send_in_call(
        sip_core::Method::Info,
        &[
            ("Info-Package", fcp_core::VIEW_INFO_PACKAGE),
            ("Content-Type", fcp_core::VIEW_INFO_TYPE),
        ],
        view.as_bytes(),
    )
    .await
    .unwrap()
}

/// The streams the room says it forwards to `pid`.
async fn forwarded_streams(stack: &Stack, pid: &str) -> Vec<serde_json::Value> {
    stack.participant(pid).await["video"]["forwarded"]["streams"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// A meeting that forwards (`video_mode = auto`), in VP8, with `extra`.
async fn forwarding_meeting(stack: &Stack, extra: serde_json::Value) {
    let mut meeting = serde_json::json!({
        "video_enabled": true,
        "video_codecs": ["VP8"],
        "video_mode": "auto",
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        meeting[k] = v;
    }
    stack.seed.meeting_with(ROOM, meeting).await;
}

/// A three-rid sender in a forwarding room (`docs/FORWARDING_ROOMS.md`
/// R3b): the viewer's stream of her moves between her layers — down to
/// what its view asks, up when it asks for more, down at once when her
/// largest layer stops — and all of it under the one SSRC its roster
/// names her by.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_three_layer_sender_is_moved_between_her_layers_under_one_ssrc() {
    let Some(stack) = Stack::start_conference("", 60, VIDEO).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    forwarding_meeting(&stack, serde_json::json!({})).await;
    let alice = stack.phone("alice").await;
    alice.set_video_simulcast(&[("h", 720), ("m", 360), ("l", 180)]);
    let bob = stack.phone("bob").await;
    let alice_call = join_forwarding(&stack, &alice, true).await;
    let bob_call = join_forwarding(&stack, &bob, true).await;
    stack.wait_participants(2).await;
    let alice_id = stack.participant_named("alice").await;
    let bob_id = stack.participant_named("bob").await;

    // Bob asks for Alice alone at 180p: her small layer.
    let view = |height: u32| {
        format!(r#"{{"streams":[{{"participant":"{alice_id}","max_height":{height}}}]}}"#)
    };
    assert_eq!(send_view(&bob_call, &view(180)).await, 200);
    let deadline = tokio::time::Instant::now() + SOON;
    let ssrc = loop {
        let streams = forwarded_streams(&stack, &bob_id).await;
        if let [s] = streams.as_slice() {
            if s["rid"] == "l" && s["participant"] == alice_id.as_str() {
                break s["ssrc"].as_u64().unwrap() as u32;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Bob's one stream never took Alice's small layer: {streams:?}"
        );
        alice_call.video().unwrap().send_frames(1, 1).await.unwrap();
        tokio::time::sleep(Duration::from_millis(33)).await;
    };
    let layers = stack.participant(&alice_id).await["video"]["layers"].clone();
    assert_eq!(layers.as_array().map(Vec::len), Some(3), "{layers}");
    pump_layers(
        &stack,
        &alice_call,
        None,
        &bob_call,
        "the small layer at Bob",
        SOON,
        |v| v.layer_by_ssrc().get(&ssrc).map(String::as_str) == Some("l"),
    )
    .await;
    let settled = bob_call.video().unwrap().received_by_ssrc();

    // He asks for 720p: up to her large layer once it has been
    // affordable a while (STREAM_UP_AFTER), on the same SSRC.
    assert_eq!(send_view(&bob_call, &view(720)).await, 200);
    pump_layers(
        &stack,
        &alice_call,
        None,
        &bob_call,
        "the large layer at Bob",
        Duration::from_secs(15),
        |v| v.layer_by_ssrc().get(&ssrc).map(String::as_str) == Some("h"),
    )
    .await;

    // Her uplink stops carrying the large layer: down to the middle one
    // at once, still the same SSRC.
    pump_layers(
        &stack,
        &alice_call,
        Some(&["m", "l"]),
        &bob_call,
        "the middle layer at Bob",
        Duration::from_secs(10),
        |v| v.layer_by_ssrc().get(&ssrc).map(String::as_str) == Some("m"),
    )
    .await;

    let after = bob_call.video().unwrap().received_by_ssrc();
    for (other, n) in &after {
        if *other != ssrc {
            assert_eq!(
                Some(n),
                settled.get(other),
                "every move on one SSRC: {settled:?} then {after:?}"
            );
        }
    }
    assert!(after[&ssrc] > settled[&ssrc]);
    for call in [&alice_call, &bob_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.finish().await;
}

/// A viewer's REMB is its budget (`docs/FORWARDING_ROOMS.md` §2.5, R3b):
/// cut below its smallest stream, every stream is paused — nothing sent,
/// the room and the roster both say so — and a REMB that covers them
/// brings them back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_remb_cut_pauses_a_viewer_s_streams_and_its_roster_says_so() {
    let Some(stack) = Stack::start_conference("", 60, VIDEO).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    forwarding_meeting(&stack, serde_json::json!({})).await;
    let alice = stack.phone("alice").await;
    let carol = stack.phone("carol").await;
    let bob = stack.phone("bob").await;
    let alice_call = join_forwarding(&stack, &alice, true).await;
    let carol_call = join_forwarding(&stack, &carol, true).await;
    let bob_call = join_forwarding(&stack, &bob, true).await;
    stack.wait_participants(3).await;
    let alice_id = stack.participant_named("alice").await;
    let carol_id = stack.participant_named("carol").await;
    let bob_id = stack.participant_named("bob").await;
    let view = format!(
        r#"{{"streams":[{{"participant":"{alice_id}","priority":1}},{{"participant":"{carol_id}","priority":2}}]}}"#
    );
    assert_eq!(send_view(&bob_call, &view).await, 200);
    pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "both streams at Bob",
        |got| got.len() >= 2 && got.values().filter(|n| **n >= 3).count() >= 2,
    )
    .await;
    assert_eq!(
        bob_call.subscribe_in_call("conference", 600).await.unwrap(),
        200
    );
    let (_, full) = roster(&bob, bob_call.call_id()).await;
    assert!(!full.contains("<fcp:paused>true</fcp:paused>"), "{full}");

    // A REMB no stream fits under: both paused.
    let bob_video = bob_call.video().unwrap();
    bob_video.send_remb(1).await.unwrap();
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let p = stack.participant(&bob_id).await;
        let streams = forwarded_streams(&stack, &bob_id).await;
        if p["video"]["forwarded"]["remb_kbps"] == 1
            && streams.len() == 2
            && streams.iter().all(|s| s["paused"] == true)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never paused: {}",
            p["video"]
        );
        for call in [&alice_call, &carol_call] {
            call.video().unwrap().send_frames(1, 1).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(33)).await;
    }
    // Nothing is sent while paused.
    let paused = bob_video.received_by_ssrc();
    for _ in 0..20 {
        for call in [&alice_call, &carol_call] {
            call.video().unwrap().send_frames(1, 1).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(33)).await;
    }
    assert_eq!(bob_video.received_by_ssrc(), paused, "nothing while paused");
    // Bob's roster says so.
    let mut told = false;
    for _ in 0..10 {
        let (_, doc) = roster(&bob, bob_call.call_id()).await;
        if doc.contains("<fcp:paused>true</fcp:paused>") {
            told = true;
            break;
        }
    }
    assert!(told, "the roster never said paused");

    // A REMB that covers them: both resume.
    bob_video.send_remb(20_000).await.unwrap();
    pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "both streams back at Bob",
        |got| {
            got.iter()
                .filter(|(ssrc, n)| **n >= paused.get(ssrc).copied().unwrap_or(0) + 3)
                .count()
                >= 2
        },
    )
    .await;
    for call in [&alice_call, &carol_call, &bob_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.finish().await;
}

/// A viewer that carries transport-cc (`docs/TRANSPORT_CC.md`, R5c): the
/// room numbers what it sends him and spends what his feedback says, with
/// no REMB at all. His link narrowed to under both streams pauses the
/// second; opened again, the room's probes find the room and bring it
/// back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_viewer_s_feedback_alone_pauses_a_stream_on_a_narrow_link_and_probes_bring_it_back() {
    let Some(stack) = Stack::start_conference("", 60, VIDEO).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    forwarding_meeting(&stack, serde_json::json!({})).await;
    let alice = stack.phone("alice").await;
    let carol = stack.phone("carol").await;
    let bob = stack.phone("bob").await;
    bob.set_video_transport_cc(true);
    let alice_call = join_forwarding(&stack, &alice, true).await;
    let carol_call = join_forwarding(&stack, &carol, true).await;
    let bob_call = join_forwarding(&stack, &bob, true).await;
    stack.wait_participants(3).await;
    // About 300 kb/s each: a 1200-byte frame 30 times a second.
    for call in [&alice_call, &carol_call] {
        call.video().unwrap().set_packet_size(1200);
    }
    let alice_id = stack.participant_named("alice").await;
    let carol_id = stack.participant_named("carol").await;
    let bob_id = stack.participant_named("bob").await;
    let view = format!(
        r#"{{"streams":[{{"participant":"{alice_id}","priority":1}},{{"participant":"{carol_id}","priority":2}}]}}"#
    );
    assert_eq!(send_view(&bob_call, &view).await, 200);
    let bob_video = bob_call.video().unwrap();

    // Pump both senders until `done` holds of Bob's forwarded section.
    let until = |what: &'static str, secs: u64, done: fn(&serde_json::Value) -> bool| {
        let (stack, alice_call, carol_call, bob_id) = (&stack, &alice_call, &carol_call, &bob_id);
        async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
            loop {
                let fwd = stack.participant(bob_id).await["video"]["forwarded"].clone();
                if done(&fwd) {
                    return fwd;
                }
                if tokio::time::Instant::now() >= deadline {
                    stack.fail_with_logs(&format!("never: {what}; {fwd}")).await;
                }
                for _ in 0..3 {
                    for call in [alice_call, carol_call] {
                        call.video().unwrap().send_frames(1, 30).await.unwrap();
                    }
                    tokio::time::sleep(Duration::from_millis(33)).await;
                }
            }
        }
    };
    let playing = |f: &serde_json::Value| {
        f["streams"]
            .as_array()
            .is_some_and(|s| s.len() == 2 && s.iter().all(|s| s["paused"] == false))
    };

    let fwd = until("both streams on Bob's feedback", 20, |f| {
        f["estimate"] == "transport_cc"
            && f["streams"]
                .as_array()
                .is_some_and(|s| s.len() == 2 && s.iter().all(|s| s["paused"] == false))
    })
    .await;
    assert_eq!(fwd["remb_kbps"], 0, "no REMB: {fwd}");
    assert!(playing(&fwd));
    let (feedback, _, _) = bob_video.transport_cc_stats();
    assert!(feedback > 0, "Bob wrote feedback");

    // Narrowed to under both: the delay and loss it causes bring the
    // estimate down until Carol, the second priority, is paused.
    bob_video.shape(Some(400_000));
    let fwd = until("Carol paused on the narrow link", 30, |f| {
        f["streams"]
            .as_array()
            .is_some_and(|s| s.len() == 2 && s[0]["paused"] == false && s[1]["paused"] == true)
    })
    .await;
    assert_eq!(fwd["estimate"], "transport_cc", "{fwd}");
    let (_, dropped, _) = bob_video.transport_cc_stats();
    eprintln!("narrowed: {fwd} ({dropped} dropped)");

    // Opened again: the room probes, and Carol comes back.
    let (_, _, probes_before) = bob_video.transport_cc_stats();
    bob_video.shape(None);
    until("Carol back once the room has probed", 40, |f| {
        f["streams"]
            .as_array()
            .is_some_and(|s| s.len() == 2 && s.iter().all(|s| s["paused"] == false))
    })
    .await;
    let (_, _, probes) = bob_video.transport_cc_stats();
    assert!(probes > probes_before, "the way back was probed");

    for call in [&alice_call, &carol_call, &bob_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.finish().await;
}

/// A recorded forwarding room (`docs/FORWARDING_ROOMS.md` §8 Q9, R3b):
/// every caller is forwarded, but the recording takes the composite, so
/// the room still decodes and encodes for it.
#[cfg(all(feature = "recording-mp4", feature = "video-openh264"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_recorded_forwarding_room_still_composes_for_its_recording() {
    let Some(stack) = Stack::start_conference("", 60, VIDEO).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    forwarding_meeting(&stack, serde_json::json!({})).await;
    let alice = stack.phone("alice").await;
    let bob = stack.phone("bob").await;
    let alice_call = join_forwarding(&stack, &alice, true).await;
    let bob_call = join_forwarding(&stack, &bob, true).await;
    stack.wait_participants(2).await;
    let alice_id = stack.participant_named("alice").await;
    let bob_id = stack.participant_named("bob").await;
    pump_until(
        &stack,
        &[&alice_call],
        &bob_call,
        "Alice's stream at Bob",
        |got| got.values().any(|n| *n >= 5),
    )
    .await;
    assert!(
        !forwarded_streams(&stack, &bob_id).await.is_empty(),
        "Bob is forwarded"
    );
    // Not recorded until asked: the meeting does not auto-record.
    let (code, started) = stack
        .post(&format!("/conferences/{ROOM}/recording/start"))
        .await;
    assert!(
        (200..300).contains(&code),
        "recording start: {code} {started}"
    );
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let (_, video) = stack.get(&format!("/conferences/{ROOM}/video")).await;
        let alice_video = stack.participant(&alice_id).await["video"].clone();
        if video["video"]["encoders"].as_u64().unwrap_or(0) > 0 && alice_video["decoding"] == true {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the recording's composite never ran: {video}\n{alice_video}"
        );
        alice_call.video().unwrap().send_frames(1, 1).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for call in [&alice_call, &bob_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.finish().await;
}

/// A forwarding room reached through the call manager
/// (`docs/FORWARDING_ROOMS.md` R3b): Bob's `Recv-Info: fcp-view` reaches the
/// room and its answer reaches Bob; his anchored video section carries every
/// stream the room sends him, each under its own SSRC as his roster —
/// subscribed in his call — names them; and his view INFO reaches the room.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forwarding_room_is_reached_through_the_call_manager() {
    let Some(stack) = Stack::start_conference(r#"trusted_edges = ["127.0.0.1"]"#, 60, VIDEO).await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({
                "video_enabled": true,
                "video_codecs": ["VP8"],
                "video_mode": "auto",
            }),
        )
        .await;
    let cm = stack.start_call_manager().await;
    let alice = stack.phone("alice").await;
    let carol = stack.phone("carol").await;
    let alice_call = join_forwarding(&stack, &alice, true).await;
    let carol_call = join_forwarding(&stack, &carol, true).await;
    let bob_user = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob = Phone::new(&bob_user.username, &bob_user.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(bob.register().await.unwrap(), 200);
    bob.enable_video(fcp_testkit::video::VideoMode::Plain);
    bob.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    let bob_call = bob
        .call_with_headers(ROOM, &[("Recv-Info", "fcp-content, fcp-view")])
        .await
        .unwrap();
    if let Err(e) = bob_call.answered(Duration::from_secs(10)).await {
        stack
            .fail(&format!("Bob was not admitted: {e}"), &bob)
            .await;
    }
    assert_eq!(
        bob_call.answer_header("Recv-Info").as_deref(),
        Some("fcp-view"),
        "the room's answer reached Bob"
    );
    stack.wait_participants(3).await;
    let alice_id = stack.participant_named("alice").await;
    let carol_id = stack.participant_named("carol").await;
    let bob_id = stack.participant_named("bob").await;
    assert_eq!(
        stack.participant(&bob_id).await["view_streams"],
        serde_json::json!(true)
    );

    // Three streams across the call manager's relay, each its own SSRC.
    let got = pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "three streams through the call manager",
        |got| got.len() >= 3 && got.values().all(|n| *n >= 3),
    )
    .await;
    assert_eq!(
        bob_call.subscribe_in_call("conference", 600).await.unwrap(),
        200
    );
    let (_, full) = roster(&bob, bob_call.call_id()).await;
    let alice_ssrc = src_id_of(&full, &alice_id).expect("Alice's stream named");
    let carol_ssrc = src_id_of(&full, &carol_id).expect("Carol's stream named");
    assert!(
        got.contains_key(&alice_ssrc) && got.contains_key(&carol_ssrc),
        "{got:?}\n{full}"
    );

    // His view crosses the call manager: Carol alone from now on.
    let view = format!(r#"{{"streams":[{{"participant":"{carol_id}"}}]}}"#);
    assert_eq!(
        bob_call
            .send_in_call(
                sip_core::Method::Info,
                &[
                    ("Info-Package", fcp_core::VIEW_INFO_PACKAGE),
                    ("Content-Type", fcp_core::VIEW_INFO_TYPE),
                ],
                view.as_bytes(),
            )
            .await
            .unwrap(),
        200
    );
    pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "a tick",
        |_| true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = bob_call.video().unwrap().received_by_ssrc();
    let after = pump_until(
        &stack,
        &[&alice_call, &carol_call],
        &bob_call,
        "Carol's stream going on",
        |got| {
            got.get(&carol_ssrc).copied().unwrap_or(0)
                >= before.get(&carol_ssrc).copied().unwrap_or(0) + 10
        },
    )
    .await;
    assert_eq!(
        after.get(&alice_ssrc),
        before.get(&alice_ssrc),
        "Alice's stream stopped: {before:?} then {after:?}"
    );

    for call in [&alice_call, &carol_call, &bob_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.finish().await;
}

/// Digits as RFC 2833, `#` to finish.
async fn dial_digits(call: &fcp_testkit::Call, pin: &str) {
    tokio::time::sleep(Duration::from_millis(300)).await;
    for d in pin.chars().chain(std::iter::once('#')) {
        call.media().send_dtmf(d);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// A webinar (`docs/WEBINARS.md` §2.1–2.2). The host PIN makes a host,
/// the panelist PIN a panelist, `#` alone or a guest token an attendee.
/// The stage hears itself; the audience hears the stage, from one feed
/// encoded once for all of them, and nobody hears the audience.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_webinar_s_stage_is_heard_by_attendees_who_are_not() {
    let Some(stack) = Stack::start_conference("", 60, "join_secret = \"join-s3cret\"").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({
                "kind": "webinar",
                "host_pin": "9999",
                "panelist_pin": "5555",
            }),
        )
        .await;

    let host = stack.phone("hostess").await;
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    go_live(&stack).await;
    let panelist = stack.phone("panelist").await;
    let panelist_call = join(&stack, &panelist).await;
    dial_digits(&panelist_call, "5555").await;
    // An attendee needs no PIN: # alone.
    let ann = stack.phone("ann").await;
    let ann_call = join(&stack, &ann).await;
    dial_digits(&ann_call, "").await;
    // A guest token in a webinar admits an attendee, with no PIN asked.
    let token = fcp_core::JoinToken::new(
        stack.seed.tenant_id,
        ROOM,
        "Ben",
        fcp_core::JoinRole::Guest,
        Duration::from_secs(600),
    )
    .mint(b"join-s3cret");
    let ben = stack.phone("ben").await;
    let ben_call = ben
        .call_with_headers(ROOM, &[("X-FCP-Join", &token)])
        .await
        .unwrap();
    if let Err(e) = ben_call.answered(SOON).await {
        stack
            .fail(&format!("Ben was not admitted: {e}"), &ben)
            .await;
    }
    stack.wait_participants(4).await;

    for (name, role, listener) in [
        ("hostess", "host", false),
        ("panelist", "panelist", false),
        ("ann", "attendee", true),
        ("Ben", "attendee", true),
    ] {
        let pid = stack.participant_named(name).await;
        stack
            .wait_participant(&pid, |p| p["role"] == role && p["listener"] == listener)
            .await;
    }

    // The stage is heard by everyone.
    host_call.media().play_tone(440.0, 2000);
    for (who, call) in [
        ("panelist", &panelist_call),
        ("Ann", &ann_call),
        ("Ben", &ben_call),
    ] {
        assert!(call.media().heard(440, SOON).await, "{who} hears the host");
    }
    panelist_call.media().play_tone(550.0, 2000);
    assert!(
        host_call.media().heard(550, SOON).await,
        "the host hears the panelist"
    );
    assert!(
        ann_call.media().heard(550, SOON).await,
        "Ann hears the panelist"
    );

    // Both attendees hear one feed, encoded once for the two of them.
    let (code, room) = stack.get(&format!("/conferences/{ROOM}")).await;
    assert_eq!(code, 200, "{room}");
    assert_eq!(room["audience"]["listeners"], 2, "{room}");
    let flavors = room["audience"]["flavors"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(flavors.len(), 1, "one codec, one feed: {room}");
    assert_eq!(flavors[0]["subscribers"], 2, "{room}");
    assert!(
        flavors[0]["frames_encoded"].as_u64().unwrap_or(0) > 0,
        "{room}"
    );

    // An attendee is heard by nobody.
    ann_call.media().play_tone(660.0, 2000);
    for (who, call) in [
        ("host", &host_call),
        ("panelist", &panelist_call),
        ("Ben", &ben_call),
    ] {
        assert!(
            !call.media().heard(660, Duration::from_secs(2)).await,
            "{who} hears Ann, an attendee"
        );
    }

    for call in [&host_call, &panelist_call, &ann_call, &ben_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// A webinar's video (`docs/WEBINARS.md` §2.3–2.4). The stage sends and is
/// drawn; the audience is answered sending only, watches one composite
/// of the whole room shared by all of it whatever `exclude_self` says,
/// is never drawn, and is counted on the roster, never listed, the
/// count alone an update of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_webinar_s_audience_watches_the_stage_and_is_counted_not_listed() {
    let Some(stack) =
        Stack::start_conference("", 60, &format!("join_secret = \"join-s3cret\"\n{VIDEO}")).await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({
                "kind": "webinar",
                "video_enabled": true,
                "video_codecs": ["VP8"],
                "video_exclude_self": true,
            }),
        )
        .await;
    let token = |name: &str, role: fcp_core::JoinRole| {
        fcp_core::JoinToken::new(
            stack.seed.tenant_id,
            ROOM,
            name,
            role,
            Duration::from_secs(600),
        )
        .mint(b"join-s3cret")
    };
    let join_as = |name: &'static str, role: fcp_core::JoinRole, video: bool| {
        let stack = &stack;
        let token = token(name, role);
        async move {
            let phone = stack.phone(&name.to_lowercase()).await;
            if video {
                phone.enable_video(fcp_testkit::video::VideoMode::Plain);
                phone.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
            }
            let call = phone
                .call_with_headers(ROOM, &[("X-FCP-Join", &token)])
                .await
                .expect("INVITE sent");
            if let Err(e) = call.answered(SOON).await {
                stack
                    .fail(&format!("{name} was not admitted: {e}"), &phone)
                    .await;
            }
            (phone, call)
        }
    };
    let (host, host_call) = join_as("Hostess", fcp_core::JoinRole::Host, true).await;
    go_live(&stack).await;
    let (_panelist, panelist_call) = join_as("Panelist", fcp_core::JoinRole::Panelist, true).await;
    let (_ann, ann_call) = join_as("Ann", fcp_core::JoinRole::Guest, true).await;
    let (_ben, ben_call) = join_as("Ben", fcp_core::JoinRole::Attendee, true).await;
    stack.wait_participants(4).await;
    let ann_id = stack.participant_named("Ann").await;
    let ben_id = stack.participant_named("Ben").await;
    let panelist_id = stack.participant_named("Panelist").await;

    // Answered: the stage both ways, the audience sent to only.
    let video_of = |sdp: String| sdp[sdp.find("m=video").expect("a video section")..].to_string();
    let ann_video = video_of(ann_call.answer_sdp().unwrap());
    assert!(ann_video.contains("a=sendonly"), "{ann_video}");
    assert!(!ann_video.contains("a=simulcast"), "{ann_video}");
    let host_video = video_of(host_call.answer_sdp().unwrap());
    assert!(host_video.contains("a=sendrecv"), "{host_video}");

    // The audience is sent the stage, and what an attendee sends goes
    // nowhere. (The phone counts only its own kind of packets; the room
    // says what it sent.)
    for _ in 0..15 {
        for call in [&host_call, &panelist_call, &ann_call] {
            call.video().unwrap().send_frames(1, 10).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(66)).await;
    }
    stack
        .wait_participant(&ben_id, |p| {
            p["video"]["frames_sent"].as_u64().unwrap_or(0) > 0
                && p["video"]["packets_sent"].as_u64().unwrap_or(0) > 0
        })
        .await;
    let ann = stack.participant(&ann_id).await;
    assert_eq!(ann["video"]["audience"], true, "{ann}");
    assert_eq!(ann["video"]["view"], "composite", "{ann}");
    assert!(
        ann["video"].get("decode_errors").is_none(),
        "no source: {ann}"
    );
    let panelist = stack.participant(&panelist_id).await;
    assert_eq!(panelist["video"]["audience"], false, "{panelist}");
    assert!(
        panelist["video"].get("decode_errors").is_some(),
        "a source: {panelist}"
    );

    // One composite of the whole room for both attendees; the stage's
    // own leave themselves out.
    let (_, room) = stack.get(&format!("/conferences/{ROOM}")).await;
    let video = &room["video"];
    assert_eq!(video["audience"], 2, "{video}");
    assert_eq!(video["sources"], 2, "the stage's cameras only: {video}");
    let outputs = video["outputs"].as_array().cloned().unwrap_or_default();
    let shared: Vec<_> = outputs
        .iter()
        .filter(|o| o["scope"]["kind"] == "all" && o["view"] == "composite")
        .collect();
    assert_eq!(shared.len(), 1, "{video}");
    assert_eq!(shared[0]["flavors"][0]["subscribers"], 2, "{video}");

    // The roster: the stage listed, the audience counted.
    assert_eq!(
        host_call
            .subscribe_in_call("conference", 600)
            .await
            .unwrap(),
        200
    );
    let (_, full) = roster(&host, host_call.call_id()).await;
    assert_eq!(full.matches("<user ").count(), 2, "{full}");
    assert!(full.contains("<user-count>4</user-count>"), "{full}");
    assert!(full.contains("<fcp:audience>2</fcp:audience>"), "{full}");
    assert!(
        full.contains("<roles><entry>panelist</entry></roles>"),
        "{full}"
    );

    // A third attendee is a new count and nobody new on the roster.
    let (_carl, carl_call) = join_as("Carl", fcp_core::JoinRole::Guest, false).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (_, doc) = roster(&host, host_call.call_id()).await;
        for id in [&ann_id, &ben_id] {
            assert!(
                !doc.contains(&format!("sip:{id}@")),
                "an attendee listed: {doc}"
            );
        }
        if doc.contains("<fcp:audience>3</fcp:audience>") {
            assert!(doc.contains("<user-count>5</user-count>"), "{doc}");
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the count never moved"
        );
    }

    for call in [&host_call, &panelist_call, &ann_call, &ben_call, &carl_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// An `fcp-qa` INFO from `call` with `body`: the room's answer.
async fn send_qa(call: &fcp_testkit::Call, body: &str) -> u16 {
    call.send_in_call(
        sip_core::Method::Info,
        &[
            ("Info-Package", fcp_core::QA_INFO_PACKAGE),
            ("Content-Type", fcp_core::QA_INFO_TYPE),
        ],
        body.as_bytes(),
    )
    .await
    .unwrap()
}

/// The next `fcp-qa` INFO `phone` is sent in `call_id`, a webinar's
/// phase aside (see [`phase_told`]).
async fn qa_told(phone: &Phone, call_id: &str) -> fcp_core::qa::QaMessage {
    loop {
        match any_qa_told(phone, call_id).await {
            fcp_core::qa::QaMessage::Webinar { .. } => continue,
            message => return message,
        }
    }
}

/// The next `fcp-qa` INFO `phone` is sent in `call_id`.
async fn any_qa_told(phone: &Phone, call_id: &str) -> fcp_core::qa::QaMessage {
    match phone
        .expect(SOON, |e| {
            matches!(e, PhoneEvent::Info { call_id: c, package: Some(p), .. }
                if c == call_id && p == fcp_core::QA_INFO_PACKAGE)
        })
        .await
    {
        Some(PhoneEvent::Info { body, .. }) => {
            fcp_core::qa::QaMessage::parse(body.as_bytes()).expect("an fcp-qa message")
        }
        _ => panic!("no fcp-qa INFO in {call_id}"),
    }
}

/// The next webinar phase `phone` is told in `call_id`, and whether it
/// waits in the lobby.
async fn phase_told(phone: &Phone, call_id: &str) -> (fcp_core::webinar::WebinarPhase, bool) {
    loop {
        if let fcp_core::qa::QaMessage::Webinar { phase, lobby } = any_qa_told(phone, call_id).await
        {
            return (phase, lobby);
        }
    }
}

/// Take the room's webinar live through the API, once its sitting has
/// begun (the first caller is past their PIN).
async fn go_live(stack: &Stack) {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let (code, out) = stack
            .post(&format!("/conferences/{ROOM}/webinar/live"))
            .await;
        if code == 200 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the webinar never went live: {code} {out}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Roster NOTIFYs in `call_id` until one satisfies `done`; its body.
async fn roster_until(phone: &Phone, call_id: &str, done: impl Fn(&str) -> bool) -> String {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the roster never said it in {call_id}"
        );
        let (_, body) = roster(phone, call_id).await;
        if done(&body) {
            return body;
        }
    }
}

/// The audience's voice (`docs/WEBINARS.md` §2.5). An attendee raises a
/// hand by `*4` and another by an `fcp-qa` INFO; the stage's roster lists
/// both with `fcp:hand`, an attendee's lists only their own. A host lowers
/// one, brings the other to the stage — heard, and told so — and sends
/// them back; lowering every hand takes down what is left.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attendee_raises_a_hand_and_a_host_brings_them_to_the_stage() {
    let stack = stack_or_skip!();
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "kind": "webinar", "host_pin": "9999" }),
        )
        .await;
    let host = stack.phone("hostess").await;
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    go_live(&stack).await;
    // Ann's client takes the package; Ben is a plain phone.
    let ann = stack.phone("ann").await;
    let ann_call = ann
        .call_with_headers(ROOM, &[("Recv-Info", fcp_core::QA_INFO_PACKAGE)])
        .await
        .unwrap();
    if let Err(e) = ann_call.answered(SOON).await {
        stack
            .fail(&format!("Ann was not admitted: {e}"), &ann)
            .await;
    }
    assert!(
        ann_call
            .answer_header("Recv-Info")
            .is_some_and(|v| v.contains(fcp_core::QA_INFO_PACKAGE)),
        "the room says it will tell Ann"
    );
    dial_digits(&ann_call, "").await;
    let ben = stack.phone("ben").await;
    let ben_call = join(&stack, &ben).await;
    dial_digits(&ben_call, "").await;
    stack.wait_participants(3).await;
    let host_id = stack.participant_named("hostess").await;
    let ann_id = stack.participant_named("ann").await;
    let ben_id = stack.participant_named("ben").await;
    for pid in [&ann_id, &ben_id] {
        stack
            .wait_participant(pid, |p| p["role"] == "attendee" && p["listener"] == true)
            .await;
    }

    assert_eq!(
        host_call
            .subscribe_in_call("conference", 600)
            .await
            .unwrap(),
        200
    );
    let full = roster_until(&host, host_call.call_id(), |b| {
        b.contains("<fcp:audience>2</fcp:audience>")
    })
    .await;
    assert!(!full.contains(&ann_id) && !full.contains(&ben_id), "{full}");

    // Ben presses *4.
    ben_call.media().send_dtmf('*');
    tokio::time::sleep(Duration::from_millis(250)).await;
    ben_call.media().send_dtmf('4');
    stack
        .wait_participant(&ben_id, |p| p["hand"].is_string())
        .await;
    let doc = roster_until(&host, host_call.call_id(), |b| b.contains(&ben_id)).await;
    assert!(doc.contains("<fcp:hand>"), "{doc}");
    assert!(doc.contains("<entry>attendee</entry>"), "{doc}");

    // Ann raises hers by INFO; nothing comes back to her for it.
    assert_eq!(
        send_qa(&ann_call, r#"{"op":"hand","raised":true}"#).await,
        200
    );
    stack
        .wait_participant(&ann_id, |p| p["hand"].is_string())
        .await;
    roster_until(&host, host_call.call_id(), |b| b.contains(&ann_id)).await;
    // Her own roster shows her hand and nobody else's.
    assert_eq!(
        ann_call.subscribe_in_call("conference", 600).await.unwrap(),
        200
    );
    let hers = roster_until(&ann, ann_call.call_id(), |b| b.contains(&ann_id)).await;
    assert!(!hers.contains(&ben_id), "{hers}");
    assert!(hers.contains("<fcp:audience>1</fcp:audience>"), "{hers}");
    // What only the room says, and what is not a message, are refused.
    assert_eq!(send_qa(&ann_call, r#"{"op":"stage","on":true}"#).await, 403);
    assert_eq!(send_qa(&ann_call, r#"{"op":"wave"}"#).await, 400);
    // A host has no hand to raise.
    assert_eq!(
        send_qa(&host_call, r#"{"op":"hand","raised":true}"#).await,
        403
    );

    // The host lowers Ben's.
    let (code, out) = stack
        .send(
            "DELETE",
            &format!("/conferences/{ROOM}/participants/{ben_id}/hand"),
            None,
        )
        .await;
    assert_eq!(
        (code, &out["changed"]),
        (200, &serde_json::json!(true)),
        "{out}"
    );
    stack
        .wait_participant(&ben_id, |p| p["hand"].is_null())
        .await;

    // Ann is brought to the stage: her hand comes down, she is told, and
    // the room hears her.
    let stage = format!("/conferences/{ROOM}/participants/{ann_id}/stage");
    let (code, out) = stack.send("POST", &stage, None).await;
    assert_eq!(
        (code, &out["changed"]),
        (200, &serde_json::json!(true)),
        "{out}"
    );
    assert_eq!(
        qa_told(&ann, ann_call.call_id()).await,
        fcp_core::qa::QaMessage::Stage { on: true },
        "the stage's word is the hand's too"
    );
    stack
        .wait_participant(&ann_id, |p| p["listener"] == false && p["hand"].is_null())
        .await;
    let on_stage = roster_until(&host, host_call.call_id(), |b| {
        b.contains(&ann_id) && b.contains("<entry>panelist</entry>")
    })
    .await;
    assert!(!on_stage.contains("<fcp:hand>"), "{on_stage}");
    ann_call.media().play_tone(660.0, 2000);
    assert!(
        host_call.media().heard(660, SOON).await,
        "the host hears Ann on the stage"
    );
    assert!(
        ben_call.media().heard(660, SOON).await,
        "the audience hears Ann on the stage"
    );
    let (code, out) = stack.send("POST", &stage, None).await;
    assert_eq!(
        (code, &out["changed"]),
        (200, &serde_json::json!(false)),
        "{out}"
    );
    let (code, _) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/participants/{host_id}/stage"),
            None,
        )
        .await;
    assert_eq!(code, 409, "the host is not an attendee");

    // Sent back: a listener again, heard by nobody.
    let (code, _) = stack.send("DELETE", &stage, None).await;
    assert_eq!(code, 200);
    assert_eq!(
        qa_told(&ann, ann_call.call_id()).await,
        fcp_core::qa::QaMessage::Stage { on: false }
    );
    stack
        .wait_participant(&ann_id, |p| p["listener"] == true)
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    ann_call.media().play_tone(770.0, 2000);
    assert!(
        !host_call.media().heard(770, Duration::from_secs(2)).await,
        "the host no longer hears Ann"
    );

    // Ben raises it again; every hand comes down at once.
    ben_call.media().send_dtmf('*');
    tokio::time::sleep(Duration::from_millis(250)).await;
    ben_call.media().send_dtmf('4');
    stack
        .wait_participant(&ben_id, |p| p["hand"].is_string())
        .await;
    let (code, out) = stack
        .send("DELETE", &format!("/conferences/{ROOM}/hands"), None)
        .await;
    assert_eq!(
        (code, &out["lowered"]),
        (200, &serde_json::json!(1)),
        "{out}"
    );
    stack
        .wait_participant(&ben_id, |p| p["hand"].is_null())
        .await;

    // An app names itself by its call.
    let (code, out) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/hand"),
            Some(&format!(
                r#"{{"call_id":"{}","raised":true}}"#,
                ann_call.call_id()
            )),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["participant"], serde_json::json!(ann_id));
    assert_eq!(
        qa_told(&ann, ann_call.call_id()).await,
        fcp_core::qa::QaMessage::Hand { raised: true },
        "told, since the app is not the call"
    );
    let (code, _) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/hand"),
            Some(r#"{"call_id":"no-such-call","raised":true}"#),
        )
        .await;
    assert_eq!(code, 404);

    for call in [&host_call, &ann_call, &ben_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// The audience's voice through the call manager (`docs/WEBINARS.md`
/// §2.5): Bob's `Recv-Info: fcp-qa` reaches the room and its answer
/// reaches Bob, his hand INFO reaches the room, the room's word that he is
/// on the stage reaches him, and so do his question and the room's word
/// of it, and his chat to the hosts (a webinar's default) and the room's
/// word of it (W3b).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attendee_s_hand_and_the_stage_cross_the_call_manager() {
    let Some(stack) = Stack::start_conference(r#"trusted_edges = ["127.0.0.1"]"#, 60, "").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    // A webinar that asks no PIN: everyone who dials in is an attendee.
    stack
        .seed
        .meeting_with(ROOM, serde_json::json!({ "kind": "webinar" }))
        .await;
    let cm = stack.start_call_manager().await;
    let bob_user = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob = Phone::new(&bob_user.username, &bob_user.password, REALM, cm)
        .await
        .unwrap();
    assert_eq!(bob.register().await.unwrap(), 200);
    let bob_call = bob
        .call_with_headers(
            ROOM,
            &[
                ("Recv-Info", fcp_core::QA_INFO_PACKAGE),
                ("Recv-Info", fcp_core::CHAT_INFO_PACKAGE),
            ],
        )
        .await
        .unwrap();
    if let Err(e) = bob_call.answered(SOON).await {
        stack
            .fail(&format!("Bob was not admitted: {e}"), &bob)
            .await;
    }
    assert!(
        bob_call
            .answer_header("Recv-Info")
            .is_some_and(|v| v.contains(fcp_core::QA_INFO_PACKAGE)),
        "the room's answer reached Bob"
    );
    // Bob waits in the lobby until the webinar goes live.
    go_live(&stack).await;
    stack.wait_participants(1).await;
    let bob_id = stack.participant_named("bob").await;
    stack
        .wait_participant(&bob_id, |p| {
            p["role"] == "attendee" && p["listener"] == true
        })
        .await;

    assert_eq!(
        send_qa(&bob_call, r#"{"op":"hand","raised":true}"#).await,
        200
    );
    stack
        .wait_participant(&bob_id, |p| p["hand"].is_string())
        .await;

    let (code, out) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/participants/{bob_id}/stage"),
            None,
        )
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(
        qa_told(&bob, bob_call.call_id()).await,
        fcp_core::qa::QaMessage::Stage { on: true }
    );
    stack
        .wait_participant(&bob_id, |p| p["listener"] == false && p["hand"].is_null())
        .await;

    // A question crosses too: Bob's INFO to the room, and the room's word
    // of it back to Bob.
    assert_eq!(
        send_qa(&bob_call, r#"{"op":"ask","text":"Can you hear me?"}"#).await,
        200
    );
    let told = questions_until(&bob, bob_call.call_id(), |q| !q.is_empty()).await;
    let q = told.values().next().unwrap();
    assert!(q.mine && q.text == "Can you hear me?", "{q:?}");

    // Chat crosses as well. A webinar's attendee writes to the hosts, not
    // the room.
    assert_eq!(
        send_chat(&bob_call, r#"{"op":"post","text":"Hi","to":"room"}"#).await,
        403
    );
    assert_eq!(
        send_chat(&bob_call, r#"{"op":"post","text":"Hi","to":"hosts"}"#).await,
        200
    );
    let told = chat_until(&bob, bob_call.call_id(), |m, _| !m.is_empty()).await;
    let m = told.values().next().unwrap();
    assert!(m.mine && m.text == "Hi", "{m:?}");

    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// `fcp-qa` `questions` INFOs `phone` is sent in `call_id` until the
/// questions it has been told of satisfy `done` (later news of a question
/// replacing earlier); those questions, by id.
async fn questions_until(
    phone: &Phone,
    call_id: &str,
    done: impl Fn(&std::collections::BTreeMap<String, fcp_core::qa::QuestionView>) -> bool,
) -> std::collections::BTreeMap<String, fcp_core::qa::QuestionView> {
    let mut seen = std::collections::BTreeMap::new();
    let deadline = tokio::time::Instant::now() + SOON;
    while !done(&seen) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never told it in {call_id}: {seen:?}"
        );
        if let fcp_core::qa::QaMessage::Questions { questions } = qa_told(phone, call_id).await {
            for q in questions {
                seen.insert(q.id.clone(), q);
            }
        }
    }
    seen
}

/// Questions (`docs/WEBINARS.md` §2.5, W3.2). In a moderated webinar Ann
/// asks by INFO and Ben anonymously through the room's API: each is told
/// of their own, the host of both, and nobody else of either until the
/// host publishes Ann's. Ben upvotes it and Ann is told; nobody upvotes
/// their own, and an attendee moderates nothing. The host answers by INFO
/// and dismisses Ben's through the API, and the rows are kept.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_audience_asks_and_a_host_moderates_the_questions() {
    use fcp_core::qa::QuestionStatus;
    let stack = stack_or_skip!();
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "kind": "webinar", "host_pin": "9999", "qa_anonymous": true }),
        )
        .await;
    let qa = [("Recv-Info", fcp_core::QA_INFO_PACKAGE)];
    let host = stack.phone("hostess").await;
    let host_call = host.call_with_headers(ROOM, &qa).await.unwrap();
    if let Err(e) = host_call.answered(SOON).await {
        stack
            .fail(&format!("the host was not admitted: {e}"), &host)
            .await;
    }
    dial_digits(&host_call, "9999").await;
    go_live(&stack).await;
    let ann = stack.phone("ann").await;
    let ann_call = ann.call_with_headers(ROOM, &qa).await.unwrap();
    if let Err(e) = ann_call.answered(SOON).await {
        stack
            .fail(&format!("Ann was not admitted: {e}"), &ann)
            .await;
    }
    dial_digits(&ann_call, "").await;
    let ben = stack.phone("ben").await;
    let ben_call = ben.call_with_headers(ROOM, &qa).await.unwrap();
    if let Err(e) = ben_call.answered(SOON).await {
        stack
            .fail(&format!("Ben was not admitted: {e}"), &ben)
            .await;
    }
    dial_digits(&ben_call, "").await;
    stack.wait_participants(3).await;
    let host_id = stack.participant_named("hostess").await;
    stack
        .wait_participant(&host_id, |p| p["role"] == "host")
        .await;

    // Ann asks; the host and Ann are told, Ben sees nothing.
    assert_eq!(
        send_qa(&ann_call, r#"{"op":"ask","text":"When does it ship?"}"#).await,
        200
    );
    let hosts = questions_until(&host, host_call.call_id(), |q| q.len() == 1).await;
    let ann_q = hosts.values().next().unwrap().clone();
    assert_eq!(ann_q.text, "When does it ship?");
    assert!(ann_q.asker.is_some(), "asked with her name");
    assert!(!ann_q.published && !ann_q.mine, "{ann_q:?}");
    assert_eq!(ann_q.status, QuestionStatus::Pending);
    let anns = questions_until(&ann, ann_call.call_id(), |q| q.contains_key(&ann_q.id)).await;
    assert!(anns[&ann_q.id].mine);
    let bens = |call: &fcp_testkit::Call| {
        format!("/conferences/{ROOM}/questions?call_id={}", call.call_id())
    };
    let (code, out) = stack.send("GET", &bens(&ben_call), None).await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["questions"], serde_json::json!([]), "not published");

    // Ben asks anonymously through the room's API; his name is not on it.
    let (code, ben_q) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/questions"),
            Some(&format!(
                r#"{{"call_id":"{}","text":"Is there a recording?","anonymous":true}}"#,
                ben_call.call_id()
            )),
        )
        .await;
    assert_eq!(code, 201, "{ben_q}");
    assert!(ben_q["asker"].is_null() && ben_q["mine"] == true, "{ben_q}");
    let ben_qid = ben_q["id"].as_str().unwrap().to_string();
    questions_until(&host, host_call.call_id(), |q| q.contains_key(&ben_qid)).await;
    // Not published, so Ben cannot upvote Ann's yet.
    assert_eq!(
        send_qa(
            &ben_call,
            &format!(r#"{{"op":"upvote","id":"{}","on":true}}"#, ann_q.id)
        )
        .await,
        404
    );
    // An attendee moderates nothing.
    assert_eq!(
        send_qa(
            &ann_call,
            &format!(r#"{{"op":"publish","id":"{}"}}"#, ann_q.id)
        )
        .await,
        403
    );

    // The host publishes Ann's: Ben is told of it.
    let (code, out) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/questions/{}/publish", ann_q.id),
            None,
        )
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["published"], true);
    let bens_told = questions_until(&ben, ben_call.call_id(), |q| {
        q.get(&ann_q.id).is_some_and(|q| q.published)
    })
    .await;
    assert!(!bens_told[&ann_q.id].mine);

    // Ben upvotes it, Ann is told; nobody upvotes their own.
    assert_eq!(
        send_qa(
            &ben_call,
            &format!(r#"{{"op":"upvote","id":"{}","on":true}}"#, ann_q.id)
        )
        .await,
        200
    );
    questions_until(&ann, ann_call.call_id(), |q| {
        q.get(&ann_q.id).is_some_and(|q| q.upvotes == 1)
    })
    .await;
    assert_eq!(
        send_qa(
            &ann_call,
            &format!(r#"{{"op":"upvote","id":"{}","on":true}}"#, ann_q.id)
        )
        .await,
        403
    );

    // The host answers by INFO: both are told.
    assert_eq!(
        send_qa(
            &host_call,
            &format!(r#"{{"op":"answer","id":"{}","text":"In May."}}"#, ann_q.id)
        )
        .await,
        200
    );
    for (phone, call) in [(&ann, &ann_call), (&ben, &ben_call)] {
        let told = questions_until(phone, call.call_id(), |q| {
            q.get(&ann_q.id)
                .is_some_and(|q| q.status == QuestionStatus::Answered)
        })
        .await;
        assert_eq!(told[&ann_q.id].answer.as_deref(), Some("In May."));
    }

    // Ben's is dismissed: he is told, and it is gone from his list.
    let (code, out) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/questions/{ben_qid}/dismiss"),
            None,
        )
        .await;
    assert_eq!(code, 200, "{out}");
    questions_until(&ben, ben_call.call_id(), |q| {
        q.get(&ben_qid)
            .is_some_and(|q| q.status == QuestionStatus::Dismissed)
    })
    .await;
    let (_, out) = stack.send("GET", &bens(&ben_call), None).await;
    let listed: Vec<&str> = out["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["id"].as_str().unwrap())
        .collect();
    assert_eq!(listed, vec![ann_q.id.as_str()], "{out}");
    let (_, out) = stack
        .send("GET", &format!("/conferences/{ROOM}/questions"), None)
        .await;
    assert_eq!(out["questions"].as_array().unwrap().len(), 2, "{out}");
    let (code, _) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/questions/q-none/publish"),
            None,
        )
        .await;
    assert_eq!(code, 404);

    // Kept: the rows as they stand, upvotes counted.
    use fcp_core::qa::QuestionStore;
    let store = fcp_storage_postgres::questions::PostgresQuestionStore::new(
        stack.seed.storage().pool().clone(),
    );
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let rows = store
            .questions_of_room(&stack.seed.tenant_id, ROOM, 10)
            .await
            .unwrap();
        let ann_row = rows.iter().find(|r| r.id == ann_q.id);
        let ben_row = rows.iter().find(|r| r.id == ben_qid);
        if ann_row.is_some_and(|r| r.status == QuestionStatus::Answered && r.upvotes == 1)
            && ben_row
                .is_some_and(|r| r.status == QuestionStatus::Dismissed && r.asker_name.is_none())
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the rows never stood as the room does: {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for call in [&host_call, &ann_call, &ben_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// An `fcp-chat` INFO from `call` with `body`: the room's answer.
async fn send_chat(call: &fcp_testkit::Call, body: &str) -> u16 {
    call.send_in_call(
        sip_core::Method::Info,
        &[
            ("Info-Package", fcp_core::CHAT_INFO_PACKAGE),
            ("Content-Type", fcp_core::CHAT_INFO_TYPE),
        ],
        body.as_bytes(),
    )
    .await
    .unwrap()
}

/// `fcp-chat` INFOs `phone` is sent in `call_id` until the messages it has
/// been told of (later news of a message replacing earlier) and whether it
/// was muted satisfy `done`; those messages, by id, and the mute.
async fn chat_until(
    phone: &Phone,
    call_id: &str,
    done: impl Fn(&std::collections::BTreeMap<String, fcp_core::chat::ChatView>, bool) -> bool,
) -> std::collections::BTreeMap<String, fcp_core::chat::ChatView> {
    use fcp_core::chat::ChatMessage;
    let mut seen = std::collections::BTreeMap::new();
    let mut muted = false;
    let deadline = tokio::time::Instant::now() + SOON;
    while !done(&seen, muted) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never told it in {call_id}: {seen:?}, muted {muted}"
        );
        let told = match phone
            .expect(SOON, |e| {
                matches!(e, PhoneEvent::Info { call_id: c, package: Some(p), .. }
                    if c == call_id && p == fcp_core::CHAT_INFO_PACKAGE)
            })
            .await
        {
            Some(PhoneEvent::Info { body, .. }) => {
                ChatMessage::parse(body.as_bytes()).expect("an fcp-chat message")
            }
            _ => panic!("no fcp-chat INFO in {call_id}"),
        };
        match told {
            ChatMessage::Messages { messages } => {
                for m in messages {
                    seen.insert(m.id.clone(), m);
                }
            }
            ChatMessage::Muted { on } => muted = on,
            other => panic!("the room sent {other:?}"),
        }
    }
    seen
}

/// Chat in a meeting (`docs/WEBINARS.md` §2.5, W3b), where everyone may
/// post. Ann posts to the room by INFO and everyone who asked is told; Ben
/// writes to the hosts through the room's API and only the host is told.
/// Ann deletes nothing; the host deletes her message and everyone told of
/// it is told it is gone; the host mutes Ben, who is told and refused.
/// Carl, joining late, is sent the backlog he may see, and the rows are
/// kept with the deletion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn everyone_chats_and_a_host_deletes_and_mutes() {
    let stack = stack_or_skip!();
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "host_pin": "9999", "guest_pin": "1111", "require_guest_pin": true }),
        )
        .await;
    let chat = [("Recv-Info", fcp_core::CHAT_INFO_PACKAGE)];
    let join = |name: &'static str, pin: &'static str| {
        let stack = &stack;
        async move {
            let phone = stack.phone(name).await;
            let call = phone.call_with_headers(ROOM, &chat).await.unwrap();
            if let Err(e) = call.answered(SOON).await {
                stack
                    .fail(&format!("{name} was not admitted: {e}"), &phone)
                    .await;
            }
            assert!(
                call.answer_header("Recv-Info")
                    .is_some_and(|v| v.contains(fcp_core::CHAT_INFO_PACKAGE)),
                "the room says it sends {name} chat"
            );
            dial_digits(&call, pin).await;
            (phone, call)
        }
    };
    let (host, host_call) = join("hostess", "9999").await;
    let (ann, ann_call) = join("ann", "1111").await;
    let (ben, ben_call) = join("ben", "1111").await;
    stack.wait_participants(3).await;
    let host_id = stack.participant_named("hostess").await;
    stack
        .wait_participant(&host_id, |p| p["role"] == "host")
        .await;
    let ben_id = stack.participant_named("ben").await;

    // Ann posts to the room: all three are told.
    assert_eq!(
        send_chat(&ann_call, r#"{"op":"post","text":"Hello all"}"#).await,
        200
    );
    let mut anns_id = String::new();
    for (phone, call, mine) in [
        (&host, &host_call, false),
        (&ann, &ann_call, true),
        (&ben, &ben_call, false),
    ] {
        let told = chat_until(phone, call.call_id(), |m, _| !m.is_empty()).await;
        let m = told.values().next().unwrap();
        assert_eq!(m.text, "Hello all");
        assert_eq!(m.mine, mine, "{m:?}");
        assert!(m.sender.is_some() && !m.sender_moderates, "{m:?}");
        anns_id = m.id.clone();
    }

    // Ben writes to the hosts through the room's API: only the host is told.
    let (code, bens) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/chat"),
            Some(&format!(
                r#"{{"call_id":"{}","text":"Can you share the slides?","to":"hosts"}}"#,
                ben_call.call_id()
            )),
        )
        .await;
    assert_eq!(code, 201, "{bens}");
    assert_eq!(bens["to"], "hosts");
    let bens_id = bens["id"].as_str().unwrap().to_string();
    chat_until(&host, host_call.call_id(), |m, _| m.contains_key(&bens_id)).await;
    let as_ann = format!("/conferences/{ROOM}/chat?call_id={}", ann_call.call_id());
    let (code, out) = stack.send("GET", &as_ann, None).await;
    assert_eq!(code, 200, "{out}");
    let ids: Vec<&str> = out["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![anns_id.as_str()], "Ann does not see Ben's: {out}");

    // An attendee deletes and mutes nothing.
    assert_eq!(
        send_chat(&ann_call, &format!(r#"{{"op":"delete","id":"{anns_id}"}}"#)).await,
        403
    );
    assert_eq!(
        send_chat(
            &ann_call,
            &format!(r#"{{"op":"mute","participant_id":"{ben_id}","on":true}}"#)
        )
        .await,
        403
    );

    // The host deletes Ann's: Ann and Ben are told it is gone.
    assert_eq!(
        send_chat(
            &host_call,
            &format!(r#"{{"op":"delete","id":"{anns_id}"}}"#)
        )
        .await,
        200
    );
    for (phone, call) in [(&ann, &ann_call), (&ben, &ben_call)] {
        let told = chat_until(phone, call.call_id(), |m, _| {
            m.get(&anns_id).is_some_and(|m| m.deleted)
        })
        .await;
        assert!(told[&anns_id].text.is_empty(), "deleted without its text");
    }

    // The host mutes Ben through the API: he is told, and refused.
    let (code, out) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/participants/{ben_id}/chat-mute"),
            Some(r#"{"on":true}"#),
        )
        .await;
    assert_eq!(code, 200, "{out}");
    chat_until(&ben, ben_call.call_id(), |_, muted| muted).await;
    assert_eq!(
        send_chat(&ben_call, r#"{"op":"post","text":"Still here?"}"#).await,
        403
    );
    // A host is not muted.
    let (code, _) = stack
        .send(
            "POST",
            &format!("/conferences/{ROOM}/participants/{host_id}/chat-mute"),
            Some(r#"{"on":true}"#),
        )
        .await;
    assert_eq!(code, 403);

    // The host posts; Carl joins late and is sent what he may see: the
    // host's, not Ann's deleted one or Ben's to the hosts.
    assert_eq!(
        send_chat(&host_call, r#"{"op":"post","text":"Welcome"}"#).await,
        200
    );
    chat_until(&ann, ann_call.call_id(), |m, _| {
        m.values().any(|m| m.text == "Welcome")
    })
    .await;
    let (carl, carl_call) = join("carl", "1111").await;
    stack.wait_participants(4).await;
    let carls = chat_until(&carl, carl_call.call_id(), |m, _| !m.is_empty()).await;
    let texts: Vec<&str> = carls.values().map(|m| m.text.as_str()).collect();
    assert_eq!(texts, vec!["Welcome"], "{carls:?}");
    assert!(carls.values().all(|m| m.sender_moderates));

    // Kept: three rows, Ann's deleted by the host.
    use fcp_core::chat::ChatStore;
    let store =
        fcp_storage_postgres::chat::PostgresChatStore::new(stack.seed.storage().pool().clone());
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let rows = store
            .chat_of_room(&stack.seed.tenant_id, ROOM, 10)
            .await
            .unwrap();
        let anns_row = rows.iter().find(|r| r.id == anns_id);
        if rows.len() == 3
            && anns_row.is_some_and(|r| {
                r.deleted_at.is_some()
                    && r.deleted_by.as_deref() == Some(host_id.as_str())
                    && r.text == "Hello all"
                    && r.sender_address.is_some()
            })
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the rows never stood as the room does: {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for call in [&host_call, &ann_call, &ben_call, &carl_call] {
        assert_eq!(call.hangup().await.unwrap(), 200);
    }
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// A webinar's life (`docs/WEBINARS.md` §2.6, W4). In practice the host is
/// in the room and the attendees wait in the lobby, told so, hearing
/// nothing of the stage, and nothing is recorded. The host's `*95` takes
/// it live: the lobby is let in, told, hears the stage, and the meeting's
/// recording starts. A host ends it through the API and everyone is
/// released; a caller after that starts a new sitting. Live, a stage left
/// empty past the grace period ends the webinar by itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_webinar_practices_goes_live_and_ends() {
    let Some(stack) = Stack::start_conference("", 60, "webinar_stage_grace_secs = 1").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "kind": "webinar", "host_pin": "9999", "auto_record": true }),
        )
        .await;
    let qa = [("Recv-Info", fcp_core::QA_INFO_PACKAGE)];
    use fcp_core::webinar::WebinarPhase;

    let host = stack.phone("hostess").await;
    let host_call = host.call_with_headers(ROOM, &qa).await.unwrap();
    if let Err(e) = host_call.answered(SOON).await {
        stack
            .fail(&format!("the host was not admitted: {e}"), &host)
            .await;
    }
    dial_digits(&host_call, "9999").await;
    assert_eq!(
        phase_told(&host, host_call.call_id()).await,
        (WebinarPhase::Practice, false)
    );
    let ann = stack.phone("ann").await;
    let ann_call = ann.call_with_headers(ROOM, &qa).await.unwrap();
    if let Err(e) = ann_call.answered(SOON).await {
        stack
            .fail(&format!("Ann was not answered: {e}"), &ann)
            .await;
    }
    dial_digits(&ann_call, "").await;
    assert_eq!(
        phase_told(&ann, ann_call.call_id()).await,
        (WebinarPhase::Practice, true),
        "Ann is told she waits in the lobby"
    );
    let ben = stack.phone("ben").await;
    let ben_call = join(&stack, &ben).await;
    dial_digits(&ben_call, "").await;

    // Two in the lobby, out of the room.
    let deadline = tokio::time::Instant::now() + SOON;
    let sitting = loop {
        let (code, sitting) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
        assert_eq!(code, 200, "{sitting}");
        if sitting["lobby"] == 2 {
            break sitting;
        }
        assert!(tokio::time::Instant::now() < deadline, "{sitting}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(sitting["phase"], "practice", "{sitting}");
    assert_eq!(sitting["kind"], "webinar", "{sitting}");
    assert!(sitting["live_at"].is_null(), "{sitting}");
    let first_sitting = sitting["id"].as_str().unwrap().to_string();
    assert_eq!(stack.participants().await.len(), 1, "the host alone");
    let (_, room) = stack.get(&format!("/conferences/{ROOM}")).await;
    assert_eq!(
        room["is_recording"], false,
        "practice is not recorded: {room}"
    );
    host_call.media().play_tone(440.0, 1500);
    assert!(
        !ann_call.media().heard(440, Duration::from_secs(2)).await,
        "the lobby hears the stage"
    );

    // *95: live.
    for d in ['*', '9', '5'] {
        host_call.media().send_dtmf(d);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(
        phase_told(&ann, ann_call.call_id()).await,
        (WebinarPhase::Live, false),
        "Ann is let in and told"
    );
    stack.wait_participants(3).await;
    let (_, sitting) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_eq!(
        (&sitting["phase"], &sitting["lobby"]),
        (&serde_json::json!("live"), &serde_json::json!(0)),
        "{sitting}"
    );
    assert!(sitting["live_at"].is_string(), "{sitting}");
    let ann_id = stack.participant_named("ann").await;
    stack
        .wait_participant(&ann_id, |p| {
            p["role"] == "attendee" && p["listener"] == true
        })
        .await;
    host_call.media().play_tone(550.0, 2000);
    for (who, call) in [("Ann", &ann_call), ("Ben", &ben_call)] {
        assert!(call.media().heard(550, SOON).await, "{who} hears the host");
    }
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let (_, room) = stack.get(&format!("/conferences/{ROOM}")).await;
        if room["is_recording"] == true {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "going live did not record: {room}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Live twice is no change; a host's *95 again neither.
    let (code, out) = stack
        .post(&format!("/conferences/{ROOM}/webinar/live"))
        .await;
    assert_eq!((code, &out["changed"]), (200, &serde_json::json!(false)));

    // A host ends it: everyone is released.
    let (code, out) = stack
        .post(&format!("/conferences/{ROOM}/webinar/end"))
        .await;
    assert_eq!(code, 200, "{out}");
    assert_eq!(out["released"], 3, "{out}");
    for (who, call) in [("host", &host_call), ("Ann", &ann_call), ("Ben", &ben_call)] {
        assert!(call.ended(SOON).await, "{who} is released");
    }
    let (code, _) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_eq!(code, 404, "the sitting is over");

    // A new sitting, taken live through the API. The host leaves: past
    // the grace period, the webinar ends by itself.
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    go_live(&stack).await;
    let ben_call = join(&stack, &ben).await;
    dial_digits(&ben_call, "").await;
    stack.wait_participants(2).await;
    let (_, sitting) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_ne!(sitting["id"], first_sitting.as_str(), "{sitting}");
    assert_eq!(host_call.hangup().await.unwrap(), 200);
    assert!(
        ben_call.ended(Duration::from_secs(8)).await,
        "an empty stage ends the webinar"
    );
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// A meeting's room has a sitting too, live from its start, with no lobby,
/// and a webinar's controls are refused there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_meeting_s_sitting_is_live_and_has_no_webinar_controls() {
    let stack = stack_or_skip!();
    stack.seed.meeting(ROOM, None, None).await;
    let alice = stack.phone("alice").await;
    let call = join(&stack, &alice).await;
    stack.wait_participants(1).await;
    let (code, sitting) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_eq!(code, 200, "{sitting}");
    assert_eq!(sitting["kind"], "meeting", "{sitting}");
    assert_eq!(sitting["phase"], "live", "{sitting}");
    assert!(sitting["live_at"].is_string(), "{sitting}");
    for op in ["live", "end"] {
        let (code, out) = stack
            .post(&format!("/conferences/{ROOM}/webinar/{op}"))
            .await;
        assert_eq!(code, 409, "{op}: {out}");
    }
    assert_eq!(call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
    stack.finish().await;
}

/// Registration (`docs/WEBINARS.md` §2.6, W4.2). A webinar that requires
/// it admits an attendee only by a registrant's token: a standing
/// registration is let in, a revoked one, a guest token and the `#` of a
/// plain phone are not. The audience by page lists who is in, by their
/// registration, and their leg's record names the sitting and the
/// registration.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_webinar_that_requires_registration_admits_its_registrants_alone() {
    let Some(stack) = Stack::start_conference("", 60, "join_secret = \"join-s3cret\"").await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "kind": "webinar", "host_pin": "9999", "registration": true }),
        )
        .await;
    let ann_reg = stack.seed.registrant(ROOM, "Ann", false).await;
    let ben_reg = stack.seed.registrant(ROOM, "Ben", true).await;
    let token = |name: &str, registrant: Option<&str>| {
        let t = fcp_core::JoinToken::new(
            stack.seed.tenant_id,
            ROOM,
            name,
            fcp_core::JoinRole::Attendee,
            Duration::from_secs(600),
        );
        match registrant {
            Some(r) => t.for_registrant(r),
            None => t,
        }
        .mint(b"join-s3cret")
    };

    let host = stack.phone("hostess").await;
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    go_live(&stack).await;

    // Ann's registration stands: in.
    let ann = stack.phone("ann").await;
    let ann_call = ann
        .call_with_headers(ROOM, &[("X-FCP-Join", &token("Ann", Some(&ann_reg)))])
        .await
        .unwrap();
    if let Err(e) = ann_call.answered(SOON).await {
        stack
            .fail(&format!("Ann was not admitted: {e}"), &ann)
            .await;
    }
    // Ben's was revoked; Carl has a token naming no registration.
    for (name, registrant) in [("ben", Some(ben_reg.as_str())), ("carl", None)] {
        let phone = stack.phone(name).await;
        let call = phone
            .call_with_headers(ROOM, &[("X-FCP-Join", &token(name, registrant))])
            .await
            .unwrap();
        assert_eq!(call.failed(SOON).await, Some(403), "{name}");
    }
    // Dan dials in and presses #, as an attendee would without a token.
    let dan = stack.phone("dan").await;
    let dan_call = join(&stack, &dan).await;
    dial_digits(&dan_call, "").await;
    assert!(dan_call.ended(SOON).await, "Dan is let go");
    stack.wait_participants(2).await;

    // The audience by page: Ann, by her registration.
    let (code, page) = stack
        .get(&format!("/conferences/{ROOM}/audience?limit=10"))
        .await;
    assert_eq!(code, 200, "{page}");
    assert_eq!(page["total"], 1, "{page}");
    assert_eq!(page["data"][0]["registrant"], ann_reg.as_str(), "{page}");
    assert_eq!(page["data"][0]["lobby"], false, "{page}");
    assert!(page["next_cursor"].is_null(), "{page}");
    let (_, sitting) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    let sitting_id = sitting["id"].as_str().unwrap().to_string();

    assert_eq!(ann_call.hangup().await.unwrap(), 200);
    assert_eq!(host_call.hangup().await.unwrap(), 200);
    stack.wait_participants(0).await;
    let records = stack
        .seed
        .cdrs(
            Some(fcp_core::storage::CdrKind::Conference),
            // The host's and Ann's: Dan was let go before he joined.
            2,
            Duration::from_secs(10),
        )
        .await;
    let anns = records
        .iter()
        .filter_map(|r| r.custom_data.as_ref())
        .find(|c| c["registrant"] == ann_reg.as_str())
        .unwrap_or_else(|| panic!("no record names Ann's registration: {records:#?}"));
    assert_eq!(anns["sitting_id"], sitting_id.as_str(), "{anns}");
    assert_eq!(anns["role"], "attendee", "{anns}");
    assert_eq!(anns["lobby"], false, "{anns}");
    stack.finish().await;
}

/// A webinar pushed over RTMP (`docs/WEBINARS.md` W7.2b): nothing in
/// practice; from going live, the stage publishes to the meeting's
/// destination under its key — `onMetaData`, the AAC and AVC sequence
/// headers, a keyframe first, AAC frames, a keyframe asked for every
/// second — and the webinar's JSON says it is live. A cut connection is
/// made again, starting from a keyframe with its timestamps going on; and
/// when the webinar ends, the stream is unpublished.
#[cfg(all(feature = "rtmp", feature = "video-openh264"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_webinar_is_pushed_over_rtmp_until_it_ends() {
    use fcp_core::webinar::{RtmpDestination, RtmpDestinationStore};
    use forge_rtmp::server::StandInServer;
    let ingest = StandInServer::start().await.unwrap();
    let conference = format!("{VIDEO}\nrtmp = {{ enabled = true, keyframe_secs = 1 }}");
    let Some(stack) = Stack::start_conference("", 60, &conference).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({
                "kind": "webinar",
                "host_pin": "9999",
                "video_enabled": true,
                "video_codecs": ["VP8"],
            }),
        )
        .await;
    let meeting = fcp_core::storage::ConferenceStore::get_meeting_by_room_number(
        stack.seed.storage(),
        &stack.seed.tenant_id,
        ROOM,
    )
    .await
    .unwrap()
    .unwrap();
    RtmpDestinationStore::put_rtmp_destination(
        stack.seed.storage(),
        &RtmpDestination {
            tenant_id: meeting.tenant_id,
            meeting_id: meeting.id.to_string(),
            url: ingest.url("live2"),
            key: "abcd-efgh".to_string(),
            updated_at: std::time::SystemTime::now(),
        },
    )
    .await
    .unwrap();

    // The host's camera gives the room its video, and the push a picture.
    let host = stack.phone("hostess").await;
    host.enable_video(fcp_testkit::video::VideoMode::Plain);
    host.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    stack.wait_participants(1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        ingest.publishes().is_empty(),
        "nothing is pushed in practice"
    );

    go_live(&stack).await;
    host_call.media().play_tone(440.0, 15_000);
    let first = match ingest
        .wait_for(Duration::from_secs(20), |all| {
            all.first().is_some_and(|p| {
                p.video().filter(|t| t.body.first() == Some(&0x17)).count() >= 3
                    && p.audio().count() >= 50
            })
        })
        .await
    {
        Ok(mut all) => all.remove(0),
        Err(all) => {
            let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
            let got: Vec<_> = all
                .iter()
                .map(|p| (p.tags.len(), p.video().count(), p.audio().count()))
                .collect();
            stack
                .fail_with_logs(&format!("never pushed: {got:?}\n{w}"))
                .await
        }
    };
    assert_eq!(
        (first.app.as_str(), first.key.as_str()),
        ("live2", "abcd-efgh")
    );
    let meta = first.metadata.as_ref().expect("onMetaData");
    assert_eq!(
        meta[1].get("videocodecid").and_then(|v| v.as_number()),
        Some(7.0)
    );
    assert_eq!(
        meta[1].get("audiocodecid").and_then(|v| v.as_number()),
        Some(10.0)
    );
    let audio: Vec<_> = first.audio().collect();
    assert_eq!(
        &audio[0].body[..2],
        &[0xAF, 0],
        "the AAC sequence header first"
    );
    assert_eq!(&audio[0].body[2..], &[0x11, 0x88], "AAC-LC, 48 kHz, mono");
    assert!(audio[1..].iter().all(|t| t.body[..2] == [0xAF, 1]));
    let video: Vec<_> = first.video().collect();
    assert_eq!(
        &video[0].body[..2],
        &[0x17, 0],
        "the AVC sequence header first"
    );
    assert_eq!(&video[1].body[..2], &[0x17, 1], "then a keyframe");
    let keys: Vec<u32> = video
        .iter()
        .filter(|t| t.body[..2] == [0x17, 1])
        .map(|t| t.timestamp)
        .collect();
    let gaps: Vec<u32> = keys.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps.iter().all(|g| (500..=2_500).contains(g)),
        "a keyframe about every second: {gaps:?}"
    );
    assert!(
        video.windows(2).all(|w| w[1].timestamp >= w[0].timestamp),
        "video in order"
    );
    let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_eq!(w["rtmp"]["state"], "live", "{w}");
    assert_eq!(w["rtmp"]["connects"], 1, "{w}");
    assert_eq!(
        w["rtmp"]["ingest"],
        format!("127.0.0.1:{}", ingest.addr().port()).as_str()
    );
    assert!(
        !w.to_string().contains("abcd-efgh"),
        "the key is never shown: {w}"
    );

    // Cut: connected again, from a keyframe, the timestamps going on.
    let last_before = first.tags.iter().map(|t| t.timestamp).max().unwrap();
    ingest.cut();
    let again = ingest
        .wait_for(Duration::from_secs(20), |all| {
            all.len() >= 2
                && all[1]
                    .video()
                    .filter(|t| t.body.first() == Some(&0x17))
                    .count()
                    >= 2
        })
        .await
        .unwrap_or_else(|all| panic!("never pushed again: {all:#?}"))
        .remove(1);
    assert!(again.metadata.is_some());
    let video: Vec<_> = again.video().collect();
    assert_eq!(&video[0].body[..2], &[0x17, 0], "the sequence header again");
    assert_eq!(&video[1].body[..2], &[0x17, 1], "and a keyframe first");
    assert!(
        video[1].timestamp > last_before,
        "the timestamps go on: {} after {last_before}",
        video[1].timestamp
    );
    let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_eq!(w["rtmp"]["connects"], 2, "{w}");

    // Ended: unpublished.
    let (code, out) = stack
        .post(&format!("/conferences/{ROOM}/webinar/end"))
        .await;
    assert_eq!(code, 200, "{out}");
    ingest
        .wait_for(Duration::from_secs(10), |all| {
            all[1].unpublished && all[1].closed
        })
        .await
        .unwrap_or_else(|all| panic!("never unpublished: {all:#?}"));
    let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert!(w["rtmp"].is_null(), "{w}");
    let _ = host_call.hangup().await;
    stack.finish().await;
}

/// The push against a real RTMP server (`docs/WEBINARS.md` W7.2b): ffmpeg
/// listening takes the webinar and copies it to a file, which ffprobe reads
/// as H.264 and AAC, and which decodes whole. Skipped unless
/// `FCP_TEST_FFMPEG` is set and `ffmpeg` and `ffprobe` are on the path.
#[cfg(all(feature = "rtmp", feature = "video-openh264"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_webinar_reaches_ffmpeg_s_rtmp_server() {
    use fcp_core::webinar::{RtmpDestination, RtmpDestinationStore};
    use std::process::Command;
    if std::env::var_os("FCP_TEST_FFMPEG").is_none() {
        eprintln!("skipped: FCP_TEST_FFMPEG is not set");
        return;
    }
    let conference = format!("{VIDEO}\nrtmp = {{ enabled = true }}");
    let Some(stack) = Stack::start_conference("", 60, &conference).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({
                "kind": "webinar",
                "host_pin": "9999",
                "video_enabled": true,
                "video_codecs": ["VP8"],
            }),
        )
        .await;
    let meeting = fcp_core::storage::ConferenceStore::get_meeting_by_room_number(
        stack.seed.storage(),
        &stack.seed.tenant_id,
        ROOM,
    )
    .await
    .unwrap()
    .unwrap();
    let port = free_port();
    let out = stack._dir.path().join("pushed.flv");
    let mut server = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-listen",
            "1",
            "-i",
        ])
        .arg(format!("rtmp://127.0.0.1:{port}/live/key"))
        .args(["-c", "copy"])
        .arg(&out)
        .spawn()
        .expect("ffmpeg");
    RtmpDestinationStore::put_rtmp_destination(
        stack.seed.storage(),
        &RtmpDestination {
            tenant_id: meeting.tenant_id,
            meeting_id: meeting.id.to_string(),
            url: format!("rtmp://127.0.0.1:{port}/live"),
            key: "key".to_string(),
            updated_at: std::time::SystemTime::now(),
        },
    )
    .await
    .unwrap();
    let host = stack.phone("hostess").await;
    host.enable_video(fcp_testkit::video::VideoMode::Plain);
    host.set_video_codec(fcp_testkit::video::VideoCodec::Vp8);
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    stack.wait_participants(1).await;
    // ffmpeg may take a moment to listen; the push retries until it does.
    tokio::time::sleep(Duration::from_millis(500)).await;
    go_live(&stack).await;
    host_call.media().play_tone(440.0, 12_000);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
        if w["rtmp"]["state"] == "live" && w["rtmp"]["bytes_sent"].as_u64().unwrap_or(0) > 50_000 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "never live: {w}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    let (code, _) = stack
        .post(&format!("/conferences/{ROOM}/webinar/end"))
        .await;
    assert_eq!(code, 200);
    let ended = tokio::task::spawn_blocking(move || server.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(ended.success(), "ffmpeg took the stream to its end");
    let probe = Command::new("ffprobe")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-show_entries",
            "stream=codec_name,sample_rate,channels:format=duration",
            "-of",
            "default=nw=1",
        ])
        .arg(&out)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&probe.stdout).to_string();
    assert!(text.contains("codec_name=h264"), "{text}");
    assert!(
        text.contains("codec_name=aac") && text.contains("sample_rate=48000"),
        "{text}"
    );
    let duration: f64 = text
        .lines()
        .find_map(|l| l.strip_prefix("duration="))
        .and_then(|d| d.parse().ok())
        .unwrap_or(0.0);
    assert!(duration >= 4.0, "seconds of it: {text}");
    let decoded = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&out)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success() && decoded.stderr.is_empty(),
        "it decodes whole: {}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let _ = host_call.hangup().await;
    stack.finish().await;
}

/// A stream copied to a bucket (`docs/WEBINARS.md` §2.8, W6.3c): each file
/// put as it is written, with the content type and `Cache-Control` the node
/// serves it with; a segment the bucket holds is redirected there, through
/// `public_url`, while the playlist is always the node's; and deleting the
/// stream takes the bucket's copy too.
#[cfg(feature = "recording-mp4")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stream_is_copied_to_its_bucket_and_its_segments_sent_there() {
    use fcp_testkit::s3::{StandInS3, ACCESS_KEY_ID, SECRET_ACCESS_KEY};
    let s3 = StandInS3::start("streams").await.unwrap();
    let hls = format!(
        "hls = {{ enabled = true, segment_secs = 1, public_url = \"https://cdn.example.test/live/\", \
         s3 = {{ bucket = \"streams\", endpoint = \"{}\", path_style = true, \
         access_key_id = \"{ACCESS_KEY_ID}\", secret_access_key = \"{SECRET_ACCESS_KEY}\" }} }}",
        s3.url()
    );
    let Some(stack) = Stack::start_conference("", 60, &hls).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "kind": "webinar", "host_pin": "9999", "streaming": "hls" }),
        )
        .await;
    let host = stack.phone("hostess").await;
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    stack.wait_participants(1).await;
    go_live(&stack).await;
    host_call.media().play_tone(440.0, 8000);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let sitting = loop {
        let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
        if let Some(s) = w["stream"]["sitting"].as_str() {
            break s.to_string();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never streamed: {w}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let key = |file: &str| format!("hls/{sitting}/{file}");
    // Put as written: the init segment, segments, and the playlist after
    // the segment it names.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let listed = s3
            .object(&key("index.m3u8"))
            .map(|o| String::from_utf8(o.body).unwrap())
            .unwrap_or_default();
        if listed.matches("#EXTINF:").count() >= 2 {
            for line in listed.lines().filter(|l| l.starts_with("seg-")) {
                assert!(
                    s3.object(&key(line)).is_some(),
                    "the playlist in the bucket names {line}, which is not there yet: {:?}",
                    s3.keys()
                );
            }
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the bucket never got the stream: {:?}",
            s3.keys()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let init = s3.object(&key("init.mp4")).expect("the init segment");
    assert_eq!(init.content_type.as_deref(), Some("video/mp4"));
    assert_eq!(
        init.cache_control.as_deref(),
        Some("public, max-age=31536000, immutable")
    );
    let playlist = s3.object(&key("index.m3u8")).unwrap();
    assert_eq!(
        playlist.content_type.as_deref(),
        Some("application/vnd.apple.mpegurl")
    );
    assert_eq!(playlist.cache_control.as_deref(), Some("no-cache"));
    let seg = s3.object(&key("seg-1.m4s")).expect("the first segment");
    let dir = stack._dir.path().join("recordings/hls").join(&sitting);
    assert_eq!(seg.body, std::fs::read(dir.join("seg-1.m4s")).unwrap());

    // A segment is sent to the bucket, through its public URL; the
    // playlist never is.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let get = |path: String| {
        let client = client.clone();
        async move {
            client
                .get(loopback(stack.http, &path))
                .send()
                .await
                .unwrap()
        }
    };
    let resp = get(format!("/hls/{sitting}/seg-1.m4s")).await;
    assert_eq!(resp.status(), 302);
    assert_eq!(
        resp.headers()["location"],
        format!("https://cdn.example.test/live/hls/{sitting}/seg-1.m4s").as_str()
    );
    let resp = get(format!("/hls/{sitting}/init.mp4?node=e2e-conference")).await;
    assert_eq!(resp.status(), 302, "named by its node, the same");
    let resp = get(format!("/hls/{sitting}/index.m3u8")).await;
    assert_eq!(resp.status(), 200, "the playlist is the node's");
    let resp = get(format!("/hls/{sitting}/seg-999.m4s")).await;
    assert_eq!(
        resp.status(),
        404,
        "what the bucket does not have, the node answers"
    );

    // Ended: the bucket's playlist ends too.
    let (code, out) = stack
        .post(&format!("/conferences/{ROOM}/webinar/end"))
        .await;
    assert_eq!(code, 200, "{out}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let ended = s3
            .object(&key("index.m3u8"))
            .is_some_and(|o| o.body.ends_with(b"#EXT-X-ENDLIST\n"));
        if ended && stack.get(&format!("/conferences/{ROOM}/webinar")).await.1["stream"].is_null() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the bucket's playlist never ended"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let local: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| !n.starts_with('.'))
        .collect();
    for name in &local {
        assert!(s3.object(&key(name)).is_some(), "{name} not copied");
    }

    // Deleted, the copy goes with the directory.
    let (code, out) = stack
        .send("DELETE", &format!("/recordings/{sitting}"), None)
        .await;
    assert_eq!(code, 200, "{out}");
    assert!(!dir.exists());
    assert!(s3.keys().is_empty(), "left in the bucket: {:?}", s3.keys());
    let _ = host_call.hangup().await;
    stack.finish().await;
}

/// A webinar streamed out as HLS (`docs/WEBINARS.md` §2.8, W6.2): nothing
/// in practice; from going live, a live playlist of segments a player can
/// fetch from the node; and when the webinar ends, the playlist ends too.
/// The stream is a recording (W6.3a): its row, completed when it ends, and
/// its directory downloaded as one MP4 and deleted as one.
#[cfg(feature = "recording-mp4")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_webinar_is_streamed_as_hls_until_it_ends() {
    let Some(stack) =
        Stack::start_conference("", 60, "hls = { enabled = true, segment_secs = 1 }").await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    stack
        .seed
        .meeting_with(
            ROOM,
            serde_json::json!({ "kind": "webinar", "host_pin": "9999", "streaming": "hls" }),
        )
        .await;
    let host = stack.phone("hostess").await;
    let host_call = join(&stack, &host).await;
    dial_digits(&host_call, "9999").await;
    stack.wait_participants(1).await;
    // In practice: no stream.
    let (code, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    assert_eq!(code, 200, "{w}");
    assert!(w["stream"].is_null(), "nothing streams in practice: {w}");

    go_live(&stack).await;
    host_call.media().play_tone(440.0, 8000);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let playlist_path = loop {
        let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
        if let Some(p) = w["stream"]["playlist"].as_str() {
            assert_eq!(w["stream"]["sitting"], w["id"], "the sitting's stream: {w}");
            break p.to_string();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never streamed: {w}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let fetch = |path: String| async move {
        fcp_testkit::smoke::http_get(&loopback(stack.http, &path))
            .await
            .expect("the node answers")
    };
    // Segments of a second each, as a player sees them.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let live = loop {
        let (code, body) = fetch(playlist_path.clone()).await;
        if code == 200 && body.matches("#EXTINF:").count() >= 3 {
            break body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the playlist never grew: {code} {body}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert!(live.contains("#EXT-X-PLAYLIST-TYPE:EVENT"), "{live}");
    assert!(
        !live.contains("#EXT-X-ENDLIST"),
        "live until it ends: {live}"
    );
    let base = playlist_path.trim_end_matches("index.m3u8").to_string();
    let (code, init) = fetch(format!("{base}init.mp4")).await;
    assert_eq!(code, 200);
    assert!(init.len() > 16, "an init segment");
    let (code, seg) = fetch(format!("{base}seg-1.m4s")).await;
    assert_eq!(code, 200, "{seg}");
    let (code, _) = fetch(format!("{base}..%2Fsecret")).await;
    assert_ne!(code, 200, "only a stream's own files are served");
    // Named by its node, as the admin's stream link asks (W6.3b): this
    // node serves its own, and a node it does not know is not reached.
    let (code, named) = fetch(format!("{base}init.mp4?node=e2e-conference")).await;
    assert_eq!(code, 200);
    assert_eq!(named, init);
    let (code, _) = fetch(format!("{base}init.mp4?node=nowhere")).await;
    assert_eq!(code, 503);

    // Kept as a recording (W6.3a): a `stream` row naming the directory,
    // written before the first segment, which nobody deletes while it runs.
    use fcp_core::recording::RecordingStore as _;
    let recordings = fcp_storage_postgres::recording::PostgresRecordingStore::new(
        stack.seed.storage().pool().clone(),
    );
    let (_, w) = stack.get(&format!("/conferences/{ROOM}/webinar")).await;
    let sitting = w["stream"]["sitting"].as_str().unwrap().to_string();
    let recording_id: fcp_core::RecordingId = w["stream"]["recording_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the stream names its recording: {w}"))
        .parse()
        .unwrap();
    let row = recordings
        .get_recording(&recording_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.kind, fcp_core::RecordingKind::Stream);
    assert!(row.is_running());
    assert!(
        row.path.ends_with(&format!("/hls/{sitting}")),
        "{}",
        row.path
    );
    assert_eq!(row.file_name(), sitting);
    assert!(row.meeting_id.is_some());
    let (code, _) = stack
        .send("DELETE", &format!("/recordings/{sitting}"), None)
        .await;
    assert_eq!(code, 409, "a running stream is not deleted");

    // Ended: the playlist ends, and stays for replay.
    let (code, out) = stack
        .post(&format!("/conferences/{ROOM}/webinar/end"))
        .await;
    assert_eq!(code, 200, "{out}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (code, body) = fetch(playlist_path.clone()).await;
        if code == 200 && body.ends_with("#EXT-X-ENDLIST\n") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the playlist never ended: {code} {body}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    // Its row completed, the directory's size its own.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let row = loop {
        let row = recordings
            .get_recording(&recording_id)
            .await
            .unwrap()
            .unwrap();
        if !row.is_running() {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never completed: {row:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(row.duration_secs.unwrap_or(0) >= 1, "{row:?}");
    // Downloaded as a recording is, it is one MP4: the init segment and
    // every segment, in order.
    let dir = std::path::Path::new(&row.path);
    let mut segments: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            let n = name
                .strip_prefix("seg-")?
                .strip_suffix(".m4s")?
                .parse()
                .ok()?;
            Some((n, dir.join(name)))
        })
        .collect();
    segments.sort();
    assert!(segments.len() >= 3, "{segments:?}");
    let mut whole = std::fs::read(dir.join("init.mp4")).unwrap();
    for (_, path) in &segments {
        whole.extend(std::fs::read(path).unwrap());
    }
    let (code, downloaded) = fcp_testkit::smoke::http_get_bytes(&loopback(
        stack.http,
        &format!("/recordings/{sitting}"),
    ))
    .await
    .unwrap();
    assert_eq!(code, 200);
    assert_eq!(downloaded.len(), whole.len());
    assert!(
        downloaded == whole,
        "the stream comes down as its files, in order"
    );
    assert!(&downloaded[4..8] == b"ftyp", "an MP4");
    // Deleted as one: the directory goes.
    let (code, out) = stack
        .send("DELETE", &format!("/recordings/{sitting}"), None)
        .await;
    assert_eq!(code, 200, "{out}");
    assert!(!dir.exists());
    let _ = host_call.hangup().await;
    stack.finish().await;
}
