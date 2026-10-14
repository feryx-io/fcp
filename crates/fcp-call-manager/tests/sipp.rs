//! The call manager against SIPp, a SIP implementation that is not ours
//! (`docs/DEEP_TESTING.md` D2): registration, calls answered, refused and
//! cancelled, re-INVITE, and messages no well-behaved phone sends. Both
//! ends of a call are SIPp, so nothing of siphon-rs is on the client
//! side. Skipped without `FCP_TEST_DATABASE_URL`, or without SIPp unless
//! `FCP_SIPP_TESTS=required`.

use fcp_testkit::seed::SeededUser;
use fcp_testkit::sipp::{Sipp, SippOutcome};
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";

/// A call manager on a fresh database.
struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    _db: TestDatabase,
    alice: SeededUser,
    bob: SeededUser,
    carol: SeededUser,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    async fn start() -> Option<Self> {
        Self::start_with("").await
    }

    /// A stack whose configuration ends with `extra` (whole sections).
    async fn start_with(extra: &str) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = fcp_testkit::media::allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "sipp-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
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
call_timeout_seconds = 5

[b2bua.forking]
timeout = 3

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[logging]
level = "{log}"
format = "compact"
{extra}
"#,
            db = db.url(),
            rtp_end = rtp_start + 40,
            dir = dir.path().display(),
            // `FCP_SIPP_LOG=debug` for a failure the info log does not explain.
            log = std::env::var("FCP_SIPP_LOG").unwrap_or_else(|_| "info".into()),
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        let alice = seed.user("alice", "alice-pw", "1001").await;
        let bob = seed.user("bob", "bob-pw", "1002").await;
        let carol = seed.user("carol", "carol-pw", "1003").await;
        Some(Self {
            service: tokio::sync::Mutex::new(Some(service)),
            _db: db,
            alice,
            bob,
            carol,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            _dir: dir,
        })
    }
}

impl Stack {
    /// Panic, with SIPp's errors and messages and the node's log, unless
    /// every call of the run passed.
    async fn passed(&self, out: &SippOutcome, what: &str) {
        if out.code == Some(0) {
            return;
        }
        let logs = match self.service.lock().await.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        eprintln!("--- call manager ---\n{logs}");
        out.assert_passed(what);
    }

    /// [`Stack::passed`] for both ends of a call: either failing shows
    /// both.
    async fn both_passed(&self, uac: &SippOutcome, uas: &SippOutcome, what: &str) {
        if uac.code != Some(0) || uas.code != Some(0) {
            eprintln!(
                "--- callee ({:?}) errors ---\n{}\n--- callee messages ---\n{}",
                uas.code, uas.errors, uas.messages
            );
        }
        self.passed(uac, &format!("{what}: caller")).await;
        self.passed(uas, &format!("{what}: callee")).await;
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(code, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
    }

    /// The node's bindings for `user`.
    async fn registrations(&self, user: &str) -> Vec<serde_json::Value> {
        let regs = self.get("/registrations").await;
        regs["registrations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| {
                r["aor"].as_str().is_some_and(|a| {
                    a.starts_with(&format!("{user}@")) || a.contains(&format!(":{user}@"))
                })
            })
            .cloned()
            .collect()
    }
}

fn scenario(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/sipp")
        .join(format!("{name}.xml"))
}

/// SIPp and a stack, or `None` (skip).
async fn setup() -> Option<(Sipp, Stack)> {
    let sipp = Sipp::find()?;
    let stack = Stack::start().await?;
    Some((sipp, stack))
}

#[tokio::test]
async fn sipp_registers_with_digest() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let out = sipp
        .run(
            &scenario("register"),
            stack.sip,
            &[
                ("user", "alice"),
                ("domain", REALM),
                ("contact", "127.0.0.1:5999"),
                ("expires", "300"),
            ],
            &["-au", "alice", "-ap", "alice-pw"],
        )
        .await;
    stack.passed(&out, "register").await;
    let bindings = stack.registrations("alice").await;
    assert_eq!(bindings.len(), 1, "{bindings:?}");
}

#[tokio::test]
async fn sipp_is_refused_on_a_forged_nonce_and_a_wrong_password() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let out = sipp
        .run(
            &scenario("register_refused"),
            stack.sip,
            &[
                ("user", "alice"),
                ("domain", REALM),
                ("contact", "127.0.0.1:5999"),
            ],
            &["-au", "alice", "-ap", "not-alices-password"],
        )
        .await;
    stack.passed(&out, "refused").await;
    // Nothing was bound.
    let bindings = stack.registrations("alice").await;
    assert!(bindings.is_empty(), "alice bound: {bindings:?}");
}

impl Stack {
    /// `user` registered by SIPp with its Contact at `contact`.
    async fn register(&self, sipp: &Sipp, user: &str, password: &str, contact: SocketAddr) {
        let contact = contact.to_string();
        let out = sipp
            .run(
                &scenario("register"),
                self.sip,
                &[
                    ("user", user),
                    ("domain", REALM),
                    ("contact", &contact),
                    ("expires", "300"),
                ],
                &["-au", user, "-ap", password],
            )
            .await;
        self.passed(&out, &format!("register {user}")).await;
    }

    /// The node's live calls, once there are none (or panic after 5 s).
    async fn no_calls_up(&self, what: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let live = self.get("/calls").await;
            if live["count"].as_u64().unwrap_or(0) == 0 {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what}: calls still up: {live}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// A call from SIPp's `caller` scenario as alice to bob's extension,
    /// bob being SIPp's `callee` scenario. Returns both outcomes.
    async fn call(&self, sipp: &Sipp, caller: &str, callee: &str) -> (SippOutcome, SippOutcome) {
        self.call_within(sipp, caller, callee, Duration::from_secs(30))
            .await
    }

    /// [`Stack::call`] for a scenario that runs up to `within`.
    async fn call_within(
        &self,
        sipp: &Sipp,
        caller: &str,
        callee: &str,
        within: Duration,
    ) -> (SippOutcome, SippOutcome) {
        let (alice, bob) = (&self.alice, &self.bob);
        let timeout = format!("{}s", within.as_secs());
        let uas = sipp.start(&scenario(callee), None, &[], &["-timeout", &timeout]);
        self.register(sipp, &bob.username, &bob.password, uas.addr())
            .await;
        let uac = sipp
            .start(
                &scenario(caller),
                Some(self.sip),
                &[
                    ("user", &alice.username),
                    ("domain", REALM),
                    ("callee", &bob.extension),
                ],
                &[
                    "-au",
                    &alice.username,
                    "-ap",
                    &alice.password,
                    "-timeout",
                    &timeout,
                ],
            )
            .wait(within + Duration::from_secs(10))
            .await;
        let uas = uas.wait(Duration::from_secs(10)).await;
        (uac, uas)
    }
}

#[tokio::test]
async fn sipp_calls_sipp_answered_and_hung_up() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let (uac, uas) = stack.call(&sipp, "uac_answered", "uas_answer").await;
    stack.passed(&uac, "caller").await;
    stack.passed(&uas, "callee").await;
    stack.no_calls_up("answered").await;
}

#[tokio::test]
async fn sipp_calls_sipp_refused_busy() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let (uac, uas) = stack.call(&sipp, "uac_busy", "uas_busy").await;
    stack.passed(&uac, "caller").await;
    stack.passed(&uas, "callee").await;
    stack.no_calls_up("busy").await;
}

#[tokio::test]
async fn sipp_cancels_a_ringing_call() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let (uac, uas) = stack.call(&sipp, "uac_cancel", "uas_ring").await;
    stack.passed(&uac, "caller").await;
    stack.passed(&uas, "callee").await;
    stack.no_calls_up("cancelled").await;
}

/// CANCEL crossing the callee's 200 at the node: the caller cancels on
/// the callee's 180 while the callee answers at once (five times, it is a
/// race) or after 50 ms unless cancelled first. Whichever way it goes, an
/// answered callee is ACKed and then hung up, the node never cancels after
/// it has the 200, and nothing is left up. SIPp has a window of its own
/// between sending the 180 and the 200: a CANCEL that lands in it finds a
/// callee that has not answered, which is not the node's fault.
#[tokio::test]
async fn a_cancel_racing_the_answer_leaves_nothing_up() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    // tests/sipp/scenarios.py's RACE_DELAYS.
    for (round, delay) in [0, 0, 0, 0, 0, 50].into_iter().enumerate() {
        let what = format!("round {round}, answer after {delay} ms");
        let (uac, uas) = stack
            .call(
                &sipp,
                "uac_cancel_race",
                &format!("uas_answer_after_{delay}"),
            )
            .await;
        stack.passed(&uac, &format!("caller, {what}")).await;
        if uas.sent_response(200, "INVITE") {
            stack.passed(&uas, &format!("callee, {what}")).await;
        }
        stack.no_calls_up(&what).await;
        let outcome = if uac.received_request("BYE") || uac.messages.contains("BYE sip:") {
            "answered, the caller hung up"
        } else if uas.sent_response(200, "INVITE") {
            "cancelled as the callee answered; the node hung up the callee"
        } else {
            "cancelled while ringing"
        };
        eprintln!("{what}: {outcome}");
    }
}

/// A caller who hangs up as soon as the node has the INVITE — before it is
/// authenticated and routed — is cancelled, and nobody is rung. The node
/// sends 100 first, so a CANCEL can arrive before the call exists; it used
/// to be answered 200 and forgotten, and the call put through.
#[tokio::test]
async fn a_cancel_before_the_call_is_set_up_rings_no_one() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    for round in 0..3 {
        let uas = sipp.start(&scenario("uas_answer"), None, &[], &[]);
        stack
            .register(&sipp, &stack.bob.username, &stack.bob.password, uas.addr())
            .await;
        let uac = sipp
            .run(
                &scenario("uac_cancel_early"),
                stack.sip,
                &[
                    ("user", &stack.alice.username),
                    ("domain", REALM),
                    ("callee", &stack.bob.extension),
                ],
                &["-au", &stack.alice.username, "-ap", &stack.alice.password],
            )
            .await;
        stack.passed(&uac, &format!("caller, round {round}")).await;
        let uas = uas.wait(Duration::from_secs(2)).await;
        assert!(
            !uas.received_request("INVITE"),
            "round {round}: the callee was rung for a cancelled call\n{}",
            uas.messages
        );
        stack.no_calls_up(&format!("round {round}")).await;
    }
}

#[tokio::test]
async fn sipp_reinvites_an_answered_call() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let (uac, uas) = stack.call(&sipp, "uac_reinvite", "uas_reinvite").await;
    stack.passed(&uac, "caller").await;
    stack.passed(&uas, "callee").await;
    stack.no_calls_up("re-INVITE").await;
}

/// Requests for dialogs that do not exist, from a client that is not ours:
/// each is refused as RFC 3261 says, none is taken as a call.
#[tokio::test]
async fn requests_for_no_dialog_are_refused() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let out = sipp
        .run(
            &scenario("strays"),
            stack.sip,
            &[
                ("user", &stack.alice.username),
                ("domain", REALM),
                ("callee", &stack.bob.extension),
            ],
            &[],
        )
        .await;
    stack.passed(&out, "strays").await;
    stack.no_calls_up("strays").await;
}

/// Messages no parser should accept — no SIP at all, a bad Request-URI, a
/// header without a colon, a Content-Length that lies either way, seventy
/// Vias, a 30 KB header, a stray response, a CSeq that is not a number:
/// the node takes none of them for a call and goes on answering.
#[tokio::test]
async fn messages_no_parser_should_accept_are_survived() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let out = sipp
        .run(
            &scenario("junk"),
            stack.sip,
            &[
                ("user", &stack.alice.username),
                ("domain", REALM),
                ("callee", &stack.bob.extension),
            ],
            &[],
        )
        .await;
    stack.passed(&out, "junk").await;
    // Nine broken messages and the OPTIONS, all sent.
    let sent = out.messages.matches("message sent").count();
    assert!(sent >= 10, "SIPp sent {sent} messages:\n{}", out.messages);
    stack.no_calls_up("junk").await;
    let (code, body) = http_get(&loopback(stack.http, "/health")).await.unwrap();
    assert_eq!(code, 200, "{body}");
}

/// UPDATE in an answered call (RFC 3311) is relayed to the callee and its
/// answer back.
#[tokio::test]
async fn sipp_updates_an_answered_call() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let (uac, uas) = stack.call(&sipp, "uac_update", "uas_until_bye").await;
    stack.both_passed(&uac, &uas, "call").await;
    assert!(uas.received_request("UPDATE"), "{}", uas.messages);
    stack.no_calls_up("UPDATE").await;
}

/// A callee that rings reliably (RFC 3262), to a caller that supports
/// 100rel and to one that does not: the node offers 100rel, PRACKs the
/// callee itself, and relays the ringing as an ordinary 180.
#[tokio::test]
async fn the_node_pracks_a_callee_that_rings_reliably() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    for caller in ["uac_prack", "uac_answered"] {
        let (uac, uas) = stack.call(&sipp, caller, "uas_prack").await;
        stack.both_passed(&uac, &uas, caller).await;
        stack.no_calls_up(caller).await;
    }
}

/// A blind transfer by the caller (REFER, RFC 3515), carried out by the
/// node: it dials carol back through itself, moves bob onto her, NOTIFYs
/// alice and hangs her up. Carol hangs up later and bob is hung up too.
#[tokio::test]
async fn sipp_transfers_a_call_blind() {
    let Some((sipp, stack)) = setup().await else {
        return;
    };
    let (alice, bob, carol) = (&stack.alice, &stack.bob, &stack.carol);
    let target = sipp.start(&scenario("uas_answer_then_bye"), None, &[], &[]);
    stack
        .register(&sipp, &carol.username, &carol.password, target.addr())
        .await;
    let uas = sipp.start(&scenario("uas_until_bye"), None, &[], &[]);
    stack
        .register(&sipp, &bob.username, &bob.password, uas.addr())
        .await;
    let uac = sipp
        .run(
            &scenario("uac_refer"),
            stack.sip,
            &[
                ("user", &alice.username),
                ("domain", REALM),
                ("callee", &bob.extension),
                ("target", &carol.extension),
            ],
            &["-au", &alice.username, "-ap", &alice.password],
        )
        .await;
    stack.passed(&uac, "referrer").await;
    let target = target.wait(Duration::from_secs(10)).await;
    stack.passed(&target, "target").await;
    let uas = uas.wait(Duration::from_secs(10)).await;
    stack.passed(&uas, "transferred").await;
    assert!(uac.received_request("NOTIFY"), "{}", uac.messages);
    stack.no_calls_up("transfer").await;
}

const SESSION_TIMERS: &str = r#"
[b2bua.session_timer]
enabled = true
"#;

/// An INVITE asking for less than the node's Min-SE is refused 422 with
/// the Min-SE (RFC 4028 §7.4) and rings no one.
#[tokio::test]
async fn a_session_shorter_than_min_se_is_refused_422() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    let Some(stack) = Stack::start_with(SESSION_TIMERS).await else {
        return;
    };
    let (uac, uas) = stack
        .call(&sipp, "uac_session_too_short", "uas_answer")
        .await;
    stack.passed(&uac, "caller").await;
    assert!(!uas.received_request("INVITE"), "{}", uas.messages);
    stack.no_calls_up("422").await;
}

/// A 90 s session the node refreshes: the answer keeps the caller's
/// interval and refresher (RFC 4028 §9), and the node refreshes within it.
#[tokio::test]
async fn a_session_the_node_refreshes_is_refreshed() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    let Some(stack) = Stack::start_with(SESSION_TIMERS).await else {
        return;
    };
    let (uac, uas) = stack
        .call_within(
            &sipp,
            "uac_session_refreshed",
            "uas_until_bye",
            Duration::from_secs(90),
        )
        .await;
    stack.both_passed(&uac, &uas, "call").await;
    stack.no_calls_up("refreshed").await;
}

/// A 90 s session the caller was to refresh and never does: the node ends
/// the call on both legs before the session expires (RFC 4028 §10).
#[tokio::test]
async fn a_session_nobody_refreshes_is_ended() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    let Some(stack) = Stack::start_with(SESSION_TIMERS).await else {
        return;
    };
    let (uac, uas) = stack
        .call_within(
            &sipp,
            "uac_session_expires",
            "uas_until_bye",
            Duration::from_secs(110),
        )
        .await;
    stack.both_passed(&uac, &uas, "call").await;
    stack.no_calls_up("expired").await;
}
