//! Rooms on the conference server over real SIP and RTP
//! (`docs/TEST_FLOOR.md`, T3): phones join a room directly and through
//! the call manager, hear each other through the mixer, enter a PIN,
//! mute and are muted, and leave. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{CallDisposition, CdrKind, HangupParty};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{http_get, http_post, loopback};
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
    db: Option<TestDatabase>,
    seed: Seed,
    /// The conference server's SIP address: where a phone dials a room.
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
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
node_id = "e2e-conference"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip_port}"

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

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
            rtp_end = rtp_start + 120,
            dir = dir.path().display(),
        );
        let service = Service::start("conference", BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            conference: tokio::sync::Mutex::new(Some(service)),
            call_manager: tokio::sync::Mutex::new(None),
            db: Some(db),
            seed,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            _dir: dir,
        })
    }

    /// Start a call manager on the same database, routing room numbers to
    /// this conference server; returns its SIP address.
    async fn start_call_manager(&self) -> SocketAddr {
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
        SocketAddr::from(([127, 0, 0, 1], sip_port))
    }

    /// A phone that dials the conference server directly; no registration
    /// is needed, the room number is the Request-URI.
    async fn phone(&self, name: &str) -> Phone {
        Phone::new(name, "x", REALM, self.sip)
            .await
            .expect("phone binds")
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_room_is_reached_through_the_call_manager() {
    let stack = stack_or_skip!();
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
