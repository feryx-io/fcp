//! Rooms on the conference server over real SIP and RTP
//! (`docs/TEST_FLOOR.md`, T3): phones join a room directly and through
//! the call manager, hear each other through the mixer, enter a PIN,
//! mute and are muted, and leave. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{CallDisposition, CdrKind, HangupParty};
use fcp_core::TenantId;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{http_get, http_post, http_request, loopback};
use fcp_testkit::{bins, free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-conference");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
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
        let rtp_start = allocate_port_pair().rtp_port;
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
        let rtp_start = allocate_port_pair().rtp_port;
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
