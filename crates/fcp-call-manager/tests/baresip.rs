//! The call manager between two baresip phones, a softphone that is not
//! ours, with real media (`docs/DEEP_TESTING.md` D2): each phone sends a
//! tone and the test listens to what the other heard, through codecs,
//! transcoding, DTMF and SRTP. Skipped without `FCP_TEST_DATABASE_URL`,
//! or without baresip unless `FCP_BARESIP_TESTS=required`.

use fcp_testkit::baresip::{Baresip, BaresipPhone, PhoneSetup};
use fcp_testkit::certs::ClusterCa;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback, toml_path};
use fcp_testkit::{free_port, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
/// Alice's tone, and Bob's.
const ALICE_HZ: f32 = 440.0;
const BOB_HZ: f32 = 880.0;
/// How long a call runs before its sound is judged.
const TALK: Duration = Duration::from_secs(3);

/// A call manager on a fresh database.
struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    _db: TestDatabase,
    alice: SeededUser,
    bob: SeededUser,
    sip: SocketAddr,
    /// The node's TLS listener, and the CA its certificate is from.
    tls: SocketAddr,
    ca: ClusterCa,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    async fn start() -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let tls_port = free_port();
        let http = free_port();
        let rtp_start = fcp_testkit::media::allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let ca = ClusterCa::new(dir.path());
        let cert = ca.node("baresip-call-manager");
        let config = format!(
            r#"
[service]
node_id = "baresip-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
sips_bind = "127.0.0.1:{tls_port}"
tls_cert_path = "{cert}"
tls_key_path = "{key}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
auth_invites = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 10

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "{log}"
format = "compact"
"#,
            db = db.url(),
            cert = toml_path(&cert.cert_path),
            key = toml_path(&cert.key_path),
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
            log = std::env::var("FCP_BARESIP_LOG").unwrap_or_else(|_| "info".into()),
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        let alice = seed.user("alice", "alice-pw", "1001").await;
        let bob = seed.user("bob", "bob-pw", "1002").await;
        Some(Self {
            service: tokio::sync::Mutex::new(Some(service)),
            _db: db,
            alice,
            bob,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            tls: SocketAddr::from(([127, 0, 0, 1], tls_port)),
            ca,
            http,
            _dir: dir,
        })
    }

    fn setup(&self, user: &SeededUser, tone_hz: f32) -> PhoneSetup {
        PhoneSetup::new(&user.username, &user.password, REALM, self.sip, tone_hz)
    }

    /// Panic with `why`, both phones' output and the node's log.
    async fn fail(&self, why: &str, phones: &[&BaresipPhone]) -> ! {
        for phone in phones {
            eprintln!("--- {} ---\n{}", phone.user, phone.log());
        }
        if let Some(service) = self.service.lock().await.take() {
            eprintln!("--- call manager ---\n{}", service.kill().await);
        }
        panic!("{why}");
    }

    /// Once the node holds no calls (or fail after 5 s).
    async fn no_calls_up(&self, phones: &[&BaresipPhone]) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let (code, body) = http_get(&loopback(self.http, "/calls"))
                .await
                .expect("/calls");
            assert_eq!(code, 200, "{body}");
            let live: serde_json::Value = serde_json::from_str(&body).expect("calls");
            if live["calls"].as_array().is_some_and(|c| c.is_empty()) {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                self.fail(&format!("calls still up: {live}"), phones).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Alice calls Bob, who answers; both talk for [`TALK`], each must
    /// hear the other's tone; Alice hangs up and both calls end. Returns
    /// the phones, still running, for a test that asks more of them.
    async fn call(&self, alice: &BaresipPhone, bob: &BaresipPhone) {
        let both = [alice, bob];
        if let Err(e) = alice
            .dial(&format!("sip:{}@{REALM}", self.bob.extension))
            .await
        {
            self.fail(&format!("dial: {e}"), &both).await;
        }
        for phone in both {
            if let Err(e) = phone
                .event(&["CALL_ESTABLISHED"], Duration::from_secs(10))
                .await
            {
                self.fail(&format!("{}: {e}", phone.user), &both).await;
            }
        }
        tokio::time::sleep(TALK).await;
        let window = TALK.as_millis() as usize / 2;
        if !alice.heard_tone(BOB_HZ as u32, window) {
            self.fail("Alice does not hear Bob", &both).await;
        }
        if !bob.heard_tone(ALICE_HZ as u32, window) {
            self.fail("Bob does not hear Alice", &both).await;
        }
        // The check tells tones apart: nobody hears their own.
        if alice.heard_tone(ALICE_HZ as u32, window) || bob.heard_tone(BOB_HZ as u32, window) {
            self.fail("a phone hears its own tone", &both).await;
        }
    }

    /// Alice hangs up; both see the call close and the node holds none.
    async fn hang_up(&self, alice: &BaresipPhone, bob: &BaresipPhone) {
        let both = [alice, bob];
        if let Err(e) = alice.hangup().await {
            self.fail(&format!("hangup: {e}"), &both).await;
        }
        for phone in both {
            if let Err(e) = phone.event(&["CALL_CLOSED"], Duration::from_secs(10)).await {
                self.fail(&format!("{}: {e}", phone.user), &both).await;
            }
        }
        self.no_calls_up(&both).await;
    }
}

/// baresip, a stack, and Alice's and Bob's phones as `alice` and `bob`
/// set them up (Bob answering at once), or `None` (skip).
async fn phones(
    alice: impl FnOnce(PhoneSetup) -> PhoneSetup,
    bob: impl FnOnce(PhoneSetup) -> PhoneSetup,
) -> Option<(Stack, BaresipPhone, BaresipPhone)> {
    let baresip = Baresip::find()?;
    let stack = Stack::start().await?;
    let alice = baresip
        .phone(alice(stack.setup(&stack.alice, ALICE_HZ)))
        .await
        .expect("alice's phone");
    let mut bob_setup = bob(stack.setup(&stack.bob, BOB_HZ));
    bob_setup.auto_answer = true;
    let bob = baresip.phone(bob_setup).await.expect("bob's phone");
    Some((stack, alice, bob))
}

fn codecs(setup: PhoneSetup, codecs: &[&str]) -> PhoneSetup {
    PhoneSetup {
        codecs: codecs.iter().map(|c| c.to_string()).collect(),
        ..setup
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_carries_sound_both_ways() {
    let Some((stack, alice, bob)) =
        phones(|s| codecs(s, &["PCMU"]), |s| codecs(s, &["PCMU"])).await
    else {
        return;
    };
    stack.call(&alice, &bob).await;
    stack.hang_up(&alice, &bob).await;
}

/// G.722 at both ends: wideband, carried as it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_g722_call_carries_sound_both_ways() {
    let Some((stack, alice, bob)) =
        phones(|s| codecs(s, &["G722"]), |s| codecs(s, &["G722"])).await
    else {
        return;
    };
    stack.call(&alice, &bob).await;
    stack.hang_up(&alice, &bob).await;
}

/// Opus at one end and PCMA at the other: the node transcodes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opus_and_pcma_are_transcoded() {
    let Some((stack, alice, bob)) =
        phones(|s| codecs(s, &["opus/48000/2"]), |s| codecs(s, &["PCMA"])).await
    else {
        return;
    };
    stack.call(&alice, &bob).await;
    stack.hang_up(&alice, &bob).await;
}

/// Digits Alice sends reach Bob, as RFC 4733 events (`rtpevent`) and as
/// SIP INFO (`info`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtmf_reaches_the_far_end_as_events_and_as_info() {
    for mode in ["rtpevent", "info"] {
        let with_mode = |s: PhoneSetup| PhoneSetup {
            dtmf_mode: Some(mode.into()),
            ..codecs(s, &["PCMU"])
        };
        let Some((stack, alice, bob)) = phones(with_mode, with_mode).await else {
            return;
        };
        stack.call(&alice, &bob).await;
        bob.digits_received().await;
        if let Err(e) = alice.send_digits("159#").await {
            stack
                .fail(&format!("{mode}: send digits: {e}"), &[&alice, &bob])
                .await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let got = bob.digits_received().await;
        if got != "159#" {
            stack
                .fail(
                    &format!("{mode}: Bob received {got:?}, not \"159#\""),
                    &[&alice, &bob],
                )
                .await;
        }
        stack.hang_up(&alice, &bob).await;
    }
}

/// Both phones registered over TLS, SDES SRTP required at both ends
/// (RFC 4568): the node offers SRTP only to a device whose signalling is
/// encrypted, and the call carries sound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_srtp_call_over_tls_carries_sound_both_ways() {
    let Some(baresip) = Baresip::find() else {
        return;
    };
    let Some(stack) = Stack::start().await else {
        return;
    };
    let srtp = |user: &SeededUser, tone: f32| PhoneSetup {
        media_encryption: Some("srtp-mand".into()),
        tls_ca: Some(stack.ca.ca_path.clone()),
        registrar: stack.tls,
        ..codecs(stack.setup(user, tone), &["PCMU"])
    };
    let alice = baresip
        .phone(srtp(&stack.alice, ALICE_HZ))
        .await
        .expect("alice's phone");
    let bob = baresip
        .phone(PhoneSetup {
            auto_answer: true,
            ..srtp(&stack.bob, BOB_HZ)
        })
        .await
        .expect("bob's phone");
    stack.call(&alice, &bob).await;
    stack.hang_up(&alice, &bob).await;
}

/// One note of the node's built-in hold music (`fcp_sip::hold_music`).
const DEFAULT_MUSIC_HZ: u32 = 659;

/// Alice holds: Bob hears the node's hold music instead of her, and her
/// tone again once she resumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_party_hears_music_until_resumed() {
    let Some((stack, alice, bob)) =
        phones(|s| codecs(s, &["PCMU"]), |s| codecs(s, &["PCMU"])).await
    else {
        return;
    };
    let both = [&alice, &bob];
    stack.call(&alice, &bob).await;

    if let Err(e) = alice.command("hold", "").await {
        stack.fail(&format!("hold: {e}"), &both).await;
    }
    tokio::time::sleep(TALK).await;
    let window = TALK.as_millis() as usize / 2;
    if !bob.heard_tone(DEFAULT_MUSIC_HZ, window * 2) {
        stack
            .fail("Bob, held, does not hear the hold music", &both)
            .await;
    }
    if bob.heard_tone(ALICE_HZ as u32, window) {
        stack.fail("Bob, held, still hears Alice", &both).await;
    }

    // baresip 1.1 resumes and then answers the command "Invalid
    // argument"; what Bob hears is the judge.
    if let Err(e) = alice.command("resume", "").await {
        eprintln!("resume: {e}");
    }
    tokio::time::sleep(TALK).await;
    if !bob.heard_tone(ALICE_HZ as u32, window) {
        stack
            .fail("Bob does not hear Alice after she resumes", &both)
            .await;
    }
    stack.hang_up(&alice, &bob).await;
}
