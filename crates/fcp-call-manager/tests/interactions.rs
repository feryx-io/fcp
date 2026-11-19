//! Feature interactions (`docs/DEEP_TESTING.md` D4): the enterprise
//! calling features of `docs/ENTERPRISE_CALLING.md` in combination,
//! against the binary over real SIP and RTP.
//!
//! Each row of the table below is a callee's settings (`Given`), an event
//! (`When`) and the outcome (`Then`): which phones ring first, how the
//! call ends, and which phones never ring. The outcomes were written
//! from the design's rules before the rows were run; where the design is
//! silent the row says what was chosen, and `DEEP_TESTING.md` records it.
//! The rows are chosen to cover the features pairwise, not every
//! combination.
//!
//! Every row runs on a call manager and a database of its own, with the
//! same cast: Alice places the call, Bob is the callee, Carol, Dave and
//! Erin are the others; a hoteling host, two desks on Bob's shared line,
//! and a voicemail server stand in where a row needs them. Skipped
//! without `FCP_TEST_DATABASE_URL`.

use fcp_core::storage::{
    CredentialStore, DeviceStore, LineBindingStore, PhoneNumberStore, RegistrationStore,
    VoicemailStore,
};
use fcp_core::{CallTarget, CallingSettings, CallingStore};
use fcp_testkit::certs::ClusterCa;
use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::push::{StandInApns, APNS_TEAM, APNS_TOPIC};
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{loopback, toml_path};
use fcp_testkit::{free_port, Call, Phone, Seed, Service, TestDatabase};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);
/// Long enough for a ring time or a parking orbit to run out and what
/// follows to ring.
const RING_OUT: Duration = Duration::from_secs(15);
/// Long enough to be sure a phone that should not ring did not.
const QUIET: Duration = Duration::from_secs(2);
/// A user's own ring time where a row lets it run out.
const NO_ANSWER_SECS: u32 = 5;
/// A find-me step's ring time.
const STEP_SECS: u32 = 6;
const TONE_HZ: u32 = 660;
const LONDON: &str = "+442079460000";
const LINE: &str = "+15550188";
const CLOSED_NUMBER: &str = "+15125550811";
/// A number running a flow that answers and dials Bob.
const FLOW_NUMBER: &str = "+15125550700";
const DESK_PASSWORD: &str = "desk-phone-pw";
const HOST_PASSWORD: &str = "lobby-phone-pw";

// ─── The table's words ──────────────────────────────────────────────────

/// The cast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Who {
    Alice,
    Bob,
    Carol,
    Dave,
    Erin,
}

use Who::*;

impl Who {
    const ALL: [Who; 5] = [Alice, Bob, Carol, Dave, Erin];

    fn name(self) -> &'static str {
        match self {
            Alice => "alice",
            Bob => "bob",
            Carol => "carol",
            Dave => "dave",
            Erin => "erin",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Alice => "1001",
            Bob => "1002",
            Carol => "1003",
            Dave => "1004",
            Erin => "1005",
        }
    }
}

/// A phone that may ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Ph {
    /// The user's own phone: a desk phone, or their app under `App`.
    User(Who),
    /// The hoteling host.
    Host,
    /// The two desks on Bob's shared line.
    DeskA,
    DeskB,
    /// The voicemail server.
    Voicemail,
}

/// Where a forward or a find-me step goes.
#[derive(Clone, Copy, Debug)]
enum To {
    Ext(Who),
    /// A number abroad: international, which `ClassInternal` refuses.
    London,
}

impl To {
    fn target(self) -> CallTarget {
        match self {
            To::Ext(who) => CallTarget::Extension(who.extension().into()),
            To::London => CallTarget::Number(LONDON.into()),
        }
    }

    fn find_me(self) -> String {
        match self {
            To::Ext(who) => format!("ext:{}", who.extension()),
            To::London => LONDON.into(),
        }
    }
}

/// What is set before the call.
#[derive(Clone, Copy, Debug)]
enum Given {
    Dnd(Who),
    CallWaitingOff(Who),
    ForwardAll(Who, To),
    ForwardBusy(Who, To),
    /// With the user's ring time [`NO_ANSWER_SECS`].
    ForwardNoAnswer(Who, To),
    /// One step, after the user's own phones ring [`NO_ANSWER_SECS`].
    FindMe(Who, To),
    /// Find-me with a step the user's class refuses before one it allows.
    FindMeTwo(Who, To, To),
    /// The first is the executive, the second their opted-in assistant,
    /// taking every call.
    Executive(Who, Who),
    /// A guest logged in on the host.
    Hoteling(Who),
    /// The host belongs to this user when nobody is logged in on it.
    HostOwner(Who),
    /// A shared line owned by the user, on desks A and B.
    SharedLine(Who),
    /// A class of service allowing internal calls only.
    ClassInternal(Who),
    /// A class of service allowing one call at a time.
    ClassOneCall(Who),
    /// The user's hours are never open; after hours they forward.
    AfterHours(Who, To),
    /// A number of the user's, never open, whose closed target is given.
    ClosedNumber(Who, To),
    /// The user's only phone is an app, asleep, woken by a push.
    App(Who),
    /// The user has no phone up.
    Unregistered(Who),
    /// The user has a mailbox.
    Mailbox(Who),
}

/// What Alice dials.
#[derive(Clone, Copy, Debug)]
enum Dial {
    Ext(Who),
    Line,
    ClosedNumber,
    /// The number whose flow answers and dials Bob.
    Flow,
}

/// The event.
#[derive(Clone, Copy, Debug)]
enum When {
    /// Alice dials.
    Call(Dial),
    /// Alice dials and every phone that rings first refuses (486).
    Busy(Dial),
    /// Alice dials and the phones that ring first are left ringing.
    NoAnswer(Dial),
    /// The user is on a call they placed to Erin, answered; Alice dials.
    OnACall(Who, Dial),
    /// As `OnACall`, with Erin held.
    Held(Who, Dial),
    /// The user called Erin and parked her; Alice dials.
    Parked(Who, Dial),
    /// Alice calls Erin, who transfers her (blind) to the user.
    TransferredTo(Who),
    /// The user calls Alice, parks her and is suspended; nobody takes
    /// her from the orbit.
    ParkedThenSuspended(Who),
}

/// How Alice's call ends up.
#[derive(Clone, Copy, Debug)]
enum Outcome {
    /// This phone rings, answers, and Alice hears it.
    AnsweredBy(Ph),
    /// Alice is refused, with this code when one is given.
    Refused(Option<u16>),
    /// Alice's call goes on as it was (a transfer that failed).
    Kept,
    /// Alice's call is ended by the node.
    Ended,
}

#[derive(Clone, Copy, Debug)]
struct Then {
    /// Phones that ring before the outcome, each of them.
    first: &'static [Ph],
    outcome: Outcome,
    /// Phones that never ring for Alice's call.
    silent: &'static [Ph],
}

struct Row {
    given: &'static [Given],
    when: When,
    then: Then,
    /// Why this is the outcome: the design's rule, or what was chosen.
    why: &'static str,
}

// ─── The rows ───────────────────────────────────────────────────────────

/// Run each named row as a test of its own, in parallel.
macro_rules! rows {
    ($($name:ident: $row:expr;)*) => {
        $(
            #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
            async fn $name() {
                run(stringify!($name), &$row).await;
            }
        )*
    };
}

rows! {
    // Do not disturb, against the features that ring someone else.
    dnd_beats_an_executives_screening: Row {
        given: &[Given::Executive(Bob, Carol), Given::Dnd(Bob)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1: dnd is no steps and the busy treatment; screening (§3.10.1) replaces only a plan that would ring",
    };
    dnd_with_a_busy_forward_beats_screening: Row {
        given: &[Given::Executive(Bob, Carol), Given::Dnd(Bob), Given::ForwardBusy(Bob, To::Ext(Dave))],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1: dnd forwards busy when a busy forward is set",
    };
    dnd_skips_find_me: Row {
        given: &[Given::FindMe(Bob, To::Ext(Carol)), Given::Dnd(Bob)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1: dnd comes before the steps, find-me's included",
    };
    dnd_with_a_mailbox_and_find_me_is_voicemail: Row {
        given: &[Given::FindMe(Bob, To::Ext(Carol)), Given::Dnd(Bob), Given::Mailbox(Bob)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::Voicemail), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1: dnd is forward_busy, else voicemail, else 486",
    };
    a_hoteling_guest_in_dnd_does_not_ring_the_host: Row {
        given: &[Given::Hoteling(Dave), Given::Dnd(Dave)],
        when: When::Call(Dial::Ext(Dave)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Dave), Ph::Host] },
        why: "§3.8.1: the host is one of the guest's devices, so the guest's plan decides",
    };
    a_shared_lines_owner_in_dnd_rings_no_desk: Row {
        given: &[Given::SharedLine(Bob), Given::Dnd(Bob)],
        when: When::Call(Dial::Line),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::DeskA, Ph::DeskB, Ph::User(Bob)] },
        why: "§3.9.1 deviations: the owner's plan decides whether a shared line rings",
    };

    // Forward all, against the features that ring.
    forward_all_beats_an_executives_screening: Row {
        given: &[Given::Executive(Bob, Carol), Given::ForwardAll(Bob, To::Ext(Dave))],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1: forward_all is no steps, first of every rule",
    };
    a_shared_line_forwarded_rings_no_desk: Row {
        given: &[Given::SharedLine(Bob), Given::ForwardAll(Bob, To::Ext(Carol))],
        when: When::Call(Dial::Line),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[Ph::DeskA, Ph::DeskB] },
        why: "§3.9.1: the owner's forwards apply to the line",
    };
    a_forward_reaches_a_hoteling_guest_at_the_host: Row {
        given: &[Given::ForwardAll(Bob, To::Ext(Dave)), Given::Hoteling(Dave), Given::Unregistered(Dave)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::Host), silent: &[Ph::User(Bob)] },
        why: "§3.1.1: a forward to an extension is a new call, run by Dave's plan, which rings the host he is on (§3.8.1)",
    };
    a_forward_chain_is_followed: Row {
        given: &[Given::ClosedNumber(Bob, To::Ext(Carol)), Given::ForwardAll(Carol, To::Ext(Dave))],
        when: When::Call(Dial::ClosedNumber),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.5.1: a closed number's target is placed as a call, so Carol's forward applies",
    };
    a_forward_loop_ends_without_ringing_anyone: Row {
        given: &[Given::ForwardAll(Bob, To::Ext(Carol)), Given::ForwardAll(Carol, To::Ext(Bob))],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(482)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "chosen (the design is silent): a forward reaching a user the call's forwards already reached is 482 Loop Detected; it used to place calls into the node until its own rate limit refused them",
    };
    a_no_answer_forward_loop_stops_at_the_first_user_again: Row {
        given: &[Given::ForwardNoAnswer(Bob, To::Ext(Carol)), Given::ForwardNoAnswer(Carol, To::Ext(Bob))],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob), Ph::User(Carol)], outcome: Outcome::Refused(Some(482)), silent: &[] },
        why: "as the loop above, a ring time at a time: Bob is not rung a second time",
    };
    a_forwarded_call_rings_for_the_targets_own_time: Row {
        given: &[Given::ForwardNoAnswer(Bob, To::Ext(Carol)), Given::ForwardNoAnswer(Carol, To::Ext(Dave))],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob), Ph::User(Carol)], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[] },
        why: "§3.1.1: the forward is a new call run by Carol's plan; it used to be cut off at Bob's ring time, so Carol's own treatment never applied",
    };
    a_forward_to_a_user_in_dnd_is_the_callers_answer: Row {
        given: &[Given::ForwardNoAnswer(Bob, To::Ext(Carol)), Given::Dnd(Carol)],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob)], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Carol)] },
        why: "§3.1.1: a forward that fails is the caller's answer",
    };

    // Class of service, against forwards and find-me.
    a_busy_forward_the_class_forbids_is_not_followed: Row {
        given: &[Given::ClassInternal(Bob), Given::ForwardBusy(Bob, To::London)],
        when: When::Busy(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob)], outcome: Outcome::Refused(Some(480)), silent: &[] },
        why: "§3.7.1: a forward refused by the forwarding user's class is not followed; the caller hears 480",
    };
    a_find_me_step_the_class_forbids_is_left_out: Row {
        given: &[Given::ClassInternal(Bob), Given::FindMeTwo(Bob, To::London, To::Ext(Carol))],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob)], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[] },
        why: "§3.7.1: a step's refused target is left out, the rest of the steps ring",
    };
    a_class_limit_takes_the_busy_forward: Row {
        given: &[Given::ClassOneCall(Bob), Given::ForwardBusy(Bob, To::Ext(Carol))],
        when: When::OnACall(Bob, Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[Ph::User(Bob)] },
        why: "§3.7.1: past max_calls is the busy treatment, which is the busy forward",
    };

    // Call waiting, against hold, park, transfer and the other features.
    a_held_call_still_counts_for_call_waiting: Row {
        given: &[Given::CallWaitingOff(Bob)],
        when: When::Held(Bob, Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Bob)] },
        why: "§3.4.1: a user's calls count while they are up; held is up",
    };
    a_parker_is_free_for_the_next_call: Row {
        given: &[Given::CallWaitingOff(Bob)],
        when: When::Parked(Bob, Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Bob)), silent: &[] },
        why: "§3.4.1: a park releases the parker's user",
    };
    call_waiting_off_skips_find_me: Row {
        given: &[Given::CallWaitingOff(Bob), Given::FindMe(Bob, To::Ext(Carol))],
        when: When::OnACall(Bob, Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1: on a call with call waiting off is the busy treatment, before the steps",
    };
    an_executive_on_a_call_without_call_waiting_is_busy: Row {
        given: &[Given::Executive(Bob, Carol), Given::CallWaitingOff(Bob)],
        when: When::OnACall(Bob, Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.1 and §3.10.1: busy is decided by the plan, before screening",
    };
    a_shared_lines_owner_on_a_call_without_call_waiting_is_busy: Row {
        given: &[Given::SharedLine(Bob), Given::CallWaitingOff(Bob)],
        when: When::OnACall(Bob, Dial::Line),
        then: Then { first: &[], outcome: Outcome::Refused(Some(486)), silent: &[Ph::DeskA, Ph::DeskB] },
        why: "§3.9.1 deviations: call waiting is the owner's",
    };
    a_transfer_to_a_forwarding_user_follows_the_forward: Row {
        given: &[Given::ForwardAll(Carol, To::Ext(Dave))],
        when: When::TransferredTo(Carol),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Carol)] },
        why: "§3.2.1: the transfer target is dialed back into the node and routed as any call, a user's plan included",
    };
    a_transfer_to_a_user_in_dnd_keeps_the_call: Row {
        given: &[Given::Dnd(Carol)],
        when: When::TransferredTo(Carol),
        then: Then { first: &[], outcome: Outcome::Kept, silent: &[Ph::User(Carol)] },
        why: "§3.2.1: a failed transfer is NOTIFYed and the referrer keeps the call",
    };
    a_transfer_to_an_executive_rings_the_assistant: Row {
        given: &[Given::Executive(Carol, Dave)],
        when: When::TransferredTo(Carol),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Carol)] },
        why: "§3.10.1: only an assistant's push spares a call the screening; a transfer by anyone else is screened",
    };

    // Executive-assistant, against hoteling and no answer.
    an_executives_no_answer_forward_follows_the_assistants_ringing: Row {
        given: &[Given::Executive(Bob, Carol), Given::ForwardNoAnswer(Bob, To::Ext(Dave))],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Carol)], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Bob)] },
        why: "§3.10.1: the ring time and the no-answer treatment stay the executive's",
    };
    an_assistant_logged_in_on_a_host_is_rung_there: Row {
        given: &[Given::Executive(Bob, Carol), Given::Hoteling(Carol), Given::Unregistered(Carol)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::Host), silent: &[Ph::User(Bob)] },
        why: "§3.10.1 rings the assistant's phones; §3.8.1 makes the host one of them",
    };

    // Find-me and hoteling; a lent host.
    find_me_reaches_a_hoteling_guest: Row {
        given: &[Given::FindMe(Bob, To::Ext(Dave)), Given::Hoteling(Dave)],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob)], outcome: Outcome::AnsweredBy(Ph::Host), silent: &[] },
        why: "§3.6.1: a step naming a user rings their registered contacts, the host they are on among them",
    };
    a_lent_host_does_not_ring_for_its_owner: Row {
        given: &[Given::HostOwner(Erin), Given::Hoteling(Dave)],
        when: When::Call(Dial::Ext(Erin)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Erin)), silent: &[Ph::Host] },
        why: "§3.8.1: the plan leaves a host lent to someone else out of its owner's devices",
    };

    // Schedules and push.
    an_after_hours_forward_wakes_an_app: Row {
        given: &[Given::AfterHours(Bob, To::Ext(Carol)), Given::App(Carol)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[Ph::User(Bob)] },
        why: "§3.5.1: the after-hours forward is placed as a call, and Carol's sleeping app is pushed and rung (PUSH.md P3)",
    };
    find_me_wakes_an_app: Row {
        given: &[Given::FindMe(Bob, To::Ext(Carol)), Given::App(Carol)],
        when: When::NoAnswer(Dial::Ext(Bob)),
        then: Then { first: &[Ph::User(Bob)], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[] },
        why: "§3.6.1: a step naming a user rings their registered contacts; PUSH.md P3: an app asleep among them is pushed",
    };
    an_assistants_app_is_woken: Row {
        given: &[Given::Executive(Bob, Carol), Given::App(Carol)],
        when: When::Call(Dial::Ext(Bob)),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[Ph::User(Bob)] },
        why: "§3.10.1 rings the assistant's phones; PUSH.md P3: an app asleep among them is pushed",
    };
    a_closed_numbers_target_in_dnd_takes_her_busy_forward: Row {
        given: &[Given::ClosedNumber(Bob, To::Ext(Carol)), Given::Dnd(Carol), Given::ForwardBusy(Carol, To::Ext(Dave))],
        when: When::Call(Dial::ClosedNumber),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Dave)), silent: &[Ph::User(Bob), Ph::User(Carol)] },
        why: "§3.5.1 places the closed target as a call; Carol's dnd takes her busy forward (§3.1)",
    };

    // The flow's Dial, the fourth path to a user.
    a_flow_dialing_a_forwarding_user_reaches_the_forward: Row {
        given: &[Given::ForwardAll(Bob, To::Ext(Carol))],
        when: When::Call(Dial::Flow),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[Ph::User(Bob)] },
        why: "§3.1.1: a flow's Dial to a user dials a forward's target",
    };
    a_flow_dialing_a_user_on_a_call_without_call_waiting_is_busy: Row {
        given: &[Given::CallWaitingOff(Bob)],
        when: When::OnACall(Bob, Dial::Flow),
        then: Then { first: &[], outcome: Outcome::Ended, silent: &[Ph::User(Bob)] },
        why: "§2: every path runs the plan; §3.4.1 admitted a flow's Dial did not consult call waiting",
    };
    a_flow_dialing_an_executive_rings_the_assistant: Row {
        given: &[Given::Executive(Bob, Carol)],
        when: When::Call(Dial::Flow),
        then: Then { first: &[], outcome: Outcome::AnsweredBy(Ph::User(Carol)), silent: &[Ph::User(Bob)] },
        why: "§2: every path that reaches a user runs their plan, the screening of §3.10 included",
    };

    // Suspension.
    a_parked_call_whose_parker_is_suspended_ends: Row {
        given: &[],
        when: When::ParkedThenSuspended(Bob),
        then: Then { first: &[], outcome: Outcome::Ended, silent: &[Ph::User(Bob)] },
        why: "§3.2.1: an orbit that times out is a transfer to the parker through their plan, and the call ends if it fails; USERS.md §2: a suspended user's phones are gone",
    };
}

// ─── The stack ──────────────────────────────────────────────────────────

/// A call manager over UDP and TLS, with `[push]` at the APNs stand-in
/// and a short parking orbit.
struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    db: Option<TestDatabase>,
    seed: Seed,
    udp: SocketAddr,
    tls: SocketAddr,
    voicemail: SocketAddr,
    ca: ClusterCa,
    apns: StandInApns,
    _dir: tempfile::TempDir,
}

impl Stack {
    async fn start(name: &str) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let apns = StandInApns::start().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let ca = ClusterCa::new(dir.path());
        let cert = ca.node(name);
        let apns_key = dir.path().join("apns.p8");
        std::fs::write(&apns_key, apns.config().key.unwrap()).unwrap();
        let (sip, sips, http, voicemail) = (free_port(), free_port(), free_port(), free_port());
        let rtp_start = allocate_rtp_range(60);
        let config = format!(
            r#"
[service]
node_id = "{name}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
sips_bind = "127.0.0.1:{sips}"
tls_cert_path = "{cert}"
tls_key_path = "{tls_key}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"
# A suspension's dropped registrations reach a node by event; this node
# hears none, so it reads them each time.
cache_ttl_secs = 0

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 30

[b2bua]
voicemail_server_uri = "sip:voicemail@127.0.0.1:{voicemail}"

[b2bua.forking]
timeout = 10

[pbx.parking]
enabled = true
timeout_seconds = 6

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"

[[push.apns]]
team_id = "{APNS_TEAM}"
key_id = "{key_id}"
key_file = "{key}"
topics = ["{APNS_TOPIC}"]
endpoint = "{apns_url}"

[logging]
level = "info"
format = "compact"
"#,
            db = db.url(),
            cert = toml_path(&cert.cert_path),
            tls_key = toml_path(&cert.key_path),
            dir = toml_path(dir.path()),
            rtp_end = rtp_start + 60,
            key_id = apns.config().key_id,
            key = toml_path(&apns_key),
            apns_url = apns.config().endpoint.unwrap(),
        );
        let service = Service::start(name, BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            service: tokio::sync::Mutex::new(Some(service)),
            db: Some(db),
            seed,
            udp: SocketAddr::from(([127, 0, 0, 1], sip)),
            tls: SocketAddr::from(([127, 0, 0, 1], sips)),
            voicemail: SocketAddr::from(([127, 0, 0, 1], voicemail)),
            ca,
            apns,
            _dir: dir,
        })
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

/// A row's stack, cast and phones.
struct World {
    row: &'static str,
    stack: Stack,
    users: HashMap<Who, SeededUser>,
    phones: HashMap<Ph, Arc<Phone>>,
    /// Calls the event set up, kept up until the row ends.
    kept: Vec<Call>,
}

impl World {
    async fn build(row: &'static str, given: &[Given]) -> Option<Self> {
        let stack = Stack::start(&format!("e2e-{}", row.replace('_', "-"))).await?;
        let mut users = HashMap::new();
        for who in Who::ALL {
            let user = stack
                .seed
                .user(who.name(), &format!("{}-pw", who.name()), who.extension())
                .await;
            users.insert(who, user);
        }
        let mut world = Self {
            row,
            stack,
            users,
            phones: HashMap::new(),
            kept: Vec::new(),
        };
        world.apply(given).await;
        Some(world)
    }

    fn user(&self, who: Who) -> &SeededUser {
        &self.users[&who]
    }

    async fn user_id(&self, who: Who) -> fcp_core::UserId {
        self.stack
            .seed
            .storage()
            .get_user_in_tenant(who.name(), &self.stack.seed.tenant_id)
            .await
            .expect("read user")
            .expect("seeded user")
            .id
    }

    fn phone(&self, ph: Ph) -> &Arc<Phone> {
        self.phones
            .get(&ph)
            .unwrap_or_else(|| panic!("{}: no phone {ph:?} in this row", self.row))
    }

    /// Set everything `given` says, then bring the phones up.
    async fn apply(&mut self, given: &[Given]) {
        let storage = self.stack.seed.storage();
        let tenant = self.stack.seed.tenant_id;
        let mut settings: HashMap<Who, CallingSettings> = HashMap::new();
        for who in Who::ALL {
            settings.insert(
                who,
                CallingSettings::defaults(tenant, self.user_id(who).await),
            );
        }
        let closed = {
            let schedule = fcp_core::Schedule::new(tenant, "never open", "UTC");
            fcp_core::ScheduleStore::put_schedule(storage, schedule.clone())
                .await
                .expect("seed schedule");
            schedule
        };
        let mut apps = Vec::new();
        let mut unregistered = Vec::new();
        let mut host_owner = None;
        let mut guest = None;
        let mut shared = None;
        for g in given {
            match *g {
                Given::Dnd(who) => settings.get_mut(&who).unwrap().dnd = true,
                Given::CallWaitingOff(who) => settings.get_mut(&who).unwrap().call_waiting = false,
                Given::ForwardAll(who, to) => {
                    settings.get_mut(&who).unwrap().forward_all = Some(to.target())
                }
                Given::ForwardBusy(who, to) => {
                    settings.get_mut(&who).unwrap().forward_busy = Some(to.target())
                }
                Given::ForwardNoAnswer(who, to) => {
                    let s = settings.get_mut(&who).unwrap();
                    s.forward_no_answer = Some(to.target());
                    s.no_answer_secs = Some(NO_ANSWER_SECS);
                }
                Given::FindMe(who, to) => {
                    let s = settings.get_mut(&who).unwrap();
                    s.no_answer_secs = Some(NO_ANSWER_SECS);
                    s.find_me = Some(find_me(&[&[to]]));
                }
                Given::FindMeTwo(who, first, second) => {
                    let s = settings.get_mut(&who).unwrap();
                    s.no_answer_secs = Some(NO_ANSWER_SECS);
                    s.find_me = Some(find_me(&[&[first, second]]));
                }
                Given::Executive(executive, assistant) => {
                    let assistant = self.user_id(assistant).await;
                    settings.get_mut(&executive).unwrap().executive = Some(fcp_core::Executive {
                        enabled: true,
                        filter: fcp_core::ExecutiveFilter::All,
                        alert: fcp_core::ExecutiveAlert::Assistants,
                        assistants: vec![fcp_core::Assistant {
                            user_id: assistant,
                            opted_in: true,
                        }],
                    });
                }
                Given::Hoteling(who) => guest = Some(who),
                Given::HostOwner(who) => host_owner = Some(who),
                Given::SharedLine(who) => shared = Some(who),
                Given::ClassInternal(who) => {
                    let mut class = fcp_core::ClassOfService::new(tenant, "Internal");
                    class.allowed = vec![fcp_core::DestinationClass::Internal];
                    settings.get_mut(&who).unwrap().class_of_service_id =
                        Some(self.store_class(class).await);
                }
                Given::ClassOneCall(who) => {
                    let mut class = fcp_core::ClassOfService::new(tenant, "One at a time");
                    class.max_calls = Some(1);
                    settings.get_mut(&who).unwrap().class_of_service_id =
                        Some(self.store_class(class).await);
                }
                Given::AfterHours(who, to) => {
                    let s = settings.get_mut(&who).unwrap();
                    s.after_hours_schedule_id = Some(closed.id);
                    s.after_hours_forward = Some(to.target());
                }
                Given::ClosedNumber(who, to) => {
                    self.number(who, CLOSED_NUMBER, |binding| {
                        binding.schedule_id = Some(closed.id);
                        binding.closed_target = Some(to.target());
                    })
                    .await;
                }
                Given::App(who) => apps.push(who),
                Given::Unregistered(who) => unregistered.push(who),
                Given::Mailbox(who) => self.mailbox(who).await,
            }
        }
        for (_, s) in settings {
            s.validate().expect("valid settings");
            storage
                .put_calling_settings(s)
                .await
                .expect("store settings");
        }

        self.flow_dialing_bob().await;

        // Phones: each user's own, unless the row says otherwise.
        for who in Who::ALL {
            if unregistered.contains(&who) {
                continue;
            }
            let phone = if apps.contains(&who) {
                self.sleeping_app(who).await
            } else {
                let user = self.user(who);
                let phone = Phone::new(&user.username, &user.password, REALM, self.stack.udp)
                    .await
                    .expect("phone binds");
                assert_eq!(phone.register().await.unwrap(), 200, "{who:?} registers");
                Arc::new(phone)
            };
            self.phones.insert(Ph::User(who), phone);
        }
        if guest.is_some() || host_owner.is_some() {
            self.host(host_owner, guest).await;
        }
        if let Some(owner) = shared {
            self.shared_line(owner).await;
        }
        let voicemail = Phone::bound_to(
            &self.stack.voicemail.to_string(),
            "voicemail",
            "voicemail",
            REALM,
            self.stack.udp,
        )
        .await
        .expect("voicemail stub binds");
        self.phones.insert(Ph::Voicemail, Arc::new(voicemail));
    }

    /// [`FLOW_NUMBER`]'s flow: answer, dial Bob, hang up whatever the
    /// dial's outcome.
    async fn flow_dialing_bob(&self) {
        let tenant = self.stack.seed.tenant_id;
        let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
            "id": "ring-bob",
            "name": "Ring Bob",
            "version": 1,
            "initial_node": "answer",
            "enabled": true,
            "tenant_id": tenant.to_string(),
            "nodes": {
                "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "dial"},
                "dial": {
                    "id": "dial", "name": "Dial Bob", "type": "dial",
                    "destination": Bob.extension(), "timeout": 10, "next": "done",
                    "on_error": "done", "on_no_answer": "done", "on_busy": "done"
                },
                "done": {"id": "done", "name": "Hang up", "type": "hangup"}
            }
        }))
        .expect("flow fixture deserializes");
        fcp_flow::FlowStore::insert_flow(self.stack.seed.storage(), flow)
            .await
            .expect("seed flow");
        let number = fcp_core::PhoneNumber::new(FLOW_NUMBER.into(), tenant).expect("number");
        let mut binding =
            fcp_core::LineBinding::new(number.id, None, fcp_core::BindingType::Ivr, tenant);
        binding.flow_id = Some("ring-bob".into());
        let storage = self.stack.seed.storage();
        storage.put_phone_number(number).await.expect("number");
        storage.put_line_binding(binding).await.expect("binding");
    }

    async fn store_class(&self, class: fcp_core::ClassOfService) -> fcp_core::ClassOfServiceId {
        fcp_core::ClassOfServiceStore::put_class_of_service(
            self.stack.seed.storage(),
            class.clone(),
        )
        .await
        .expect("store class");
        class.id
    }

    /// A number bound to `who`'s line and devices, as `shape` changes it.
    async fn number(&self, who: Who, e164: &str, shape: impl FnOnce(&mut fcp_core::LineBinding)) {
        let storage = self.stack.seed.storage();
        let tenant = self.stack.seed.tenant_id;
        let user = self.user_id(who).await;
        let number = fcp_core::PhoneNumber::new(e164.to_string(), tenant).expect("number");
        let mut binding =
            fcp_core::LineBinding::new(number.id, Some(user), fcp_core::BindingType::User, tenant);
        for d in storage.list_devices_by_user(&user).await.expect("devices") {
            binding.add_device(fcp_core::DeviceBinding::new(d.id));
        }
        shape(&mut binding);
        storage.put_phone_number(number).await.expect("number");
        storage.put_line_binding(binding).await.expect("binding");
    }

    async fn mailbox(&self, who: Who) {
        let storage = self.stack.seed.storage();
        let tenant = self.stack.seed.tenant_id;
        let user = self.user_id(who).await;
        let number =
            fcp_core::PhoneNumber::new(format!("+1512555{}", who.extension()), tenant).unwrap();
        let binding =
            fcp_core::LineBinding::new(number.id, Some(user), fcp_core::BindingType::User, tenant);
        let mailbox = fcp_core::VoicemailMailbox::new(
            tenant,
            user,
            binding.id,
            who.extension().to_string(),
            "1234".to_string(),
        );
        storage.put_phone_number(number).await.expect("number");
        storage.put_line_binding(binding).await.expect("binding");
        storage.put_mailbox(mailbox).await.expect("mailbox");
    }

    /// The hoteling host, owned by `owner`, with `guest` logged in on it.
    async fn host(&mut self, owner: Option<Who>, guest: Option<Who>) {
        let storage = self.stack.seed.storage();
        let tenant = self.stack.seed.tenant_id;
        let mut device = fcp_core::Device::new(
            "lobby phone".into(),
            fcp_core::DeviceType::DeskPhone,
            tenant,
        );
        device.sip_uri = Some(format!("sip:lobby@{REALM}"));
        device.sip_username = Some("lobby".into());
        device.sip_ha1_hash = Some(fcp_core::storage::password::compute_ha1(
            "lobby",
            REALM,
            HOST_PASSWORD,
        ));
        device.hoteling_host = true;
        if let Some(owner) = owner {
            device.owner_user_id = Some(self.user_id(owner).await);
        }
        storage.put_device(device.clone()).await.expect("host");
        if let Some(guest) = guest {
            let session = fcp_core::HotelingSession::start(
                tenant,
                device.id,
                self.user_id(guest).await,
                Duration::from_secs(3600),
            );
            fcp_core::HotelingStore::start_hoteling_session(storage, session)
                .await
                .expect("log the guest in");
        }
        let host = Phone::new("lobby", HOST_PASSWORD, REALM, self.stack.udp)
            .await
            .expect("host binds");
        assert_eq!(host.register().await.unwrap(), 200, "the host registers");
        self.phones.insert(Ph::Host, Arc::new(host));
    }

    /// `owner`'s shared line on two desks, each registering the line with
    /// its own credentials.
    async fn shared_line(&mut self, owner: Who) {
        let storage = self.stack.seed.storage();
        let tenant = self.stack.seed.tenant_id;
        let owner = self.user_id(owner).await;
        let number = fcp_core::PhoneNumber::new(LINE.to_string(), tenant).expect("number");
        let mut binding =
            fcp_core::LineBinding::new(number.id, Some(owner), fcp_core::BindingType::User, tenant);
        binding.shared = true;
        binding.max_appearances = 2;
        for desk in ["desk-a", "desk-b"] {
            let mut device = fcp_core::Device::new(
                format!("{desk} phone"),
                fcp_core::DeviceType::DeskPhone,
                tenant,
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
            storage.put_device(device).await.expect("desk");
        }
        storage.put_phone_number(number).await.expect("number");
        storage.put_line_binding(binding).await.expect("binding");
        for (desk, ph) in [("desk-a", Ph::DeskA), ("desk-b", Ph::DeskB)] {
            let phone = Phone::with_credentials(LINE, desk, DESK_PASSWORD, REALM, self.stack.udp)
                .await
                .expect("desk binds");
            assert_eq!(
                phone.register().await.unwrap(),
                200,
                "{desk} registers the line"
            );
            self.phones.insert(ph, Arc::new(phone));
        }
    }

    /// `who`'s app over TLS with a push address, asleep, and a task that
    /// wakes it — registers it again — when its push arrives.
    async fn sleeping_app(&self, who: Who) -> Arc<Phone> {
        let user = self.user(who);
        let app = Phone::new_app(
            &user.username,
            &user.password,
            REALM,
            self.stack.tls,
            &self.stack.ca.ca_path,
            "<urn:uuid:00000000-0000-0000-0000-0000000000a1>",
        )
        .await
        .expect("app binds");
        app.set_push(Some(&format!(
            ";pn-provider=apns;pn-param={APNS_TEAM}.{APNS_TOPIC};pn-prid=d4-{}",
            who.name()
        )))
        .unwrap();
        assert_eq!(
            app.register().await.unwrap(),
            200,
            "{who:?}'s app registers"
        );
        app.sleep().await;
        // The node notices the connection is gone.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let app = Arc::new(app);
        let mut pushes = self.stack.apns.subscribe();
        let woken = Arc::clone(&app);
        tokio::spawn(async move {
            if StandInApns::next_push(&mut pushes, 30).await.is_some() {
                let _ = woken.register().await;
            }
        });
        app
    }

    fn dial(&self, dial: Dial) -> String {
        match dial {
            Dial::Ext(who) => who.extension().to_string(),
            Dial::Line => LINE.to_string(),
            Dial::ClosedNumber => CLOSED_NUMBER.to_string(),
            Dial::Flow => FLOW_NUMBER.to_string(),
        }
    }

    /// `who` calls Erin, who answers: `who`'s end of the call.
    async fn on_a_call(&mut self, who: Who) -> Call {
        let call = self
            .phone(Ph::User(who))
            .call(Erin.extension())
            .await
            .unwrap();
        let Some(at_erin) = self.phone(Ph::User(Erin)).incoming(SOON).await else {
            self.fail(&format!("{who:?}'s call to Erin did not ring her"))
                .await;
        };
        let erin = at_erin.answer().await.unwrap();
        if let Err(e) = call.answered(SOON).await {
            self.fail(&format!("{who:?}'s call to Erin was not answered: {e}"))
                .await;
        }
        self.kept.push(erin);
        call
    }

    /// Park the far party of `call` (a transfer to the park code); the
    /// parker's leg ends.
    async fn park(&self, call: &Call) {
        assert_eq!(call.hold().await.unwrap(), 200, "{}: hold", self.row);
        let code = call
            .refer(&format!("sip:*70@{}", self.stack.udp))
            .await
            .unwrap();
        if code != 202 {
            self.fail(&format!("the park was answered {code}")).await;
        }
        if !call.ended(SOON).await {
            self.fail("the parker's leg was not dropped").await;
        }
    }

    /// Suspend `who` as the API does: the status, then every registration
    /// their phones hold (`USERS.md` §2).
    async fn suspend(&self, who: Who) {
        let storage = self.stack.seed.storage();
        let tenant = self.stack.seed.tenant_id;
        let mut user = storage
            .get_user_in_tenant(who.name(), &tenant)
            .await
            .unwrap()
            .unwrap();
        user.account_status = fcp_core::storage::AccountStatus::Suspended;
        user.normalize_status();
        storage.put_user(user).await.expect("suspend");
        for aor in [
            format!("sip:{}@{REALM}", who.name()),
            format!("sip:{}@{REALM}", who.extension()),
        ] {
            storage
                .delete_all_registrations_in_tenant(&tenant, &aor)
                .await
                .expect("drop registrations");
        }
    }

    /// Bring the event about: Alice's call, as the outcome is judged on,
    /// and whether it is answered already (a flow, a transfer, a park).
    async fn when(&mut self, when: When) -> (Call, bool) {
        let call = self.event(when).await;
        let up = match when {
            When::TransferredTo(_) | When::ParkedThenSuspended(_) => true,
            When::Call(Dial::Flow)
            | When::Busy(Dial::Flow)
            | When::NoAnswer(Dial::Flow)
            | When::OnACall(_, Dial::Flow)
            | When::Held(_, Dial::Flow)
            | When::Parked(_, Dial::Flow) => {
                if let Err(e) = call.answered(SOON).await {
                    self.fail(&format!("the flow did not answer: {e}")).await;
                }
                true
            }
            _ => false,
        };
        (call, up)
    }

    async fn event(&mut self, when: When) -> Call {
        let alice = Arc::clone(self.phone(Ph::User(Alice)));
        match when {
            When::Call(d) | When::Busy(d) | When::NoAnswer(d) => {
                alice.call(&self.dial(d)).await.unwrap()
            }
            When::OnACall(who, d) => {
                let call = self.on_a_call(who).await;
                self.kept.push(call);
                alice.call(&self.dial(d)).await.unwrap()
            }
            When::Held(who, d) => {
                let call = self.on_a_call(who).await;
                assert_eq!(call.hold().await.unwrap(), 200, "{}: hold", self.row);
                self.kept.push(call);
                alice.call(&self.dial(d)).await.unwrap()
            }
            When::Parked(who, d) => {
                let call = self.on_a_call(who).await;
                self.park(&call).await;
                alice.call(&self.dial(d)).await.unwrap()
            }
            When::TransferredTo(who) => {
                let call = alice.call(Erin.extension()).await.unwrap();
                let Some(at_erin) = self.phone(Ph::User(Erin)).incoming(SOON).await else {
                    self.fail("Alice's call did not ring Erin").await;
                };
                let erin = at_erin.answer().await.unwrap();
                if let Err(e) = call.answered(SOON).await {
                    self.fail(&format!("Erin's answer did not reach Alice: {e}"))
                        .await;
                }
                assert_eq!(erin.hold().await.unwrap(), 200, "{}: hold", self.row);
                let code = erin
                    .refer(&format!("sip:{}@{}", who.extension(), self.stack.udp))
                    .await
                    .unwrap();
                if code != 202 {
                    self.fail(&format!("the transfer was answered {code}"))
                        .await;
                }
                self.kept.push(erin);
                call
            }
            When::ParkedThenSuspended(who) => {
                let call = self
                    .phone(Ph::User(who))
                    .call(Alice.extension())
                    .await
                    .unwrap();
                let Some(at_alice) = alice.incoming(SOON).await else {
                    self.fail(&format!("{who:?}'s call did not ring Alice"))
                        .await;
                };
                let alices = at_alice.answer().await.unwrap();
                if let Err(e) = call.answered(SOON).await {
                    self.fail(&format!("Alice's answer did not reach {who:?}: {e}"))
                        .await;
                }
                self.park(&call).await;
                self.suspend(who).await;
                alices
            }
        }
    }

    /// Judge the outcome of `call` (Alice's) against `then`.
    async fn then(&mut self, when: When, then: Then, (call, up): (Call, bool)) {
        // In order: each after the ring time of the one before.
        for ph in then.first {
            let Some(incoming) = self.phone(*ph).incoming(RING_OUT).await else {
                self.fail(&format!("{ph:?} did not ring first")).await;
            };
            if matches!(when, When::Busy(_)) {
                incoming.reject(486, "Busy Here").await.unwrap();
            } else {
                self.kept.push(incoming.leave_ringing());
            }
        }
        match then.outcome {
            Outcome::AnsweredBy(ph) => {
                let Some(incoming) = self.phone(ph).incoming(RING_OUT).await else {
                    self.fail(&format!("{ph:?} did not ring")).await;
                };
                let answered = incoming.answer().await.unwrap();
                if !up {
                    if let Err(e) = call.answered(SOON).await {
                        self.fail(&format!("{ph:?} answered and Alice was not told: {e}"))
                            .await;
                    }
                }
                answered.media().play_tone(TONE_HZ as f32, 3000);
                if !call.media().heard(TONE_HZ, SOON).await {
                    self.fail(&format!("Alice does not hear {ph:?}")).await;
                }
                self.kept.push(answered);
            }
            Outcome::Refused(code) => {
                let got = call.failed(RING_OUT).await;
                let right = match code {
                    Some(code) => got == Some(code),
                    None => got.is_some(),
                };
                if !right {
                    self.fail(&format!("Alice was answered {got:?}, not {code:?}"))
                        .await;
                }
            }
            Outcome::Kept => {
                if call.ended(QUIET + QUIET).await {
                    self.fail("Alice's call was ended by a failed transfer")
                        .await;
                }
            }
            Outcome::Ended => {
                if !call.ended(RING_OUT).await {
                    self.fail("Alice's call was not ended").await;
                }
            }
        }
        tokio::time::sleep(QUIET).await;
        for ph in then.silent {
            let Some(phone) = self.phones.get(ph) else {
                continue;
            };
            if let Some(rang) = phone.incoming(Duration::from_millis(50)).await {
                self.fail(&format!("{ph:?} rang (from {})", rang.from))
                    .await;
            }
        }
        // The node is left with no call it should not have: after every
        // party hangs up, nothing stays up.
        let _ = call.hangup().await;
        for kept in self.kept.drain(..) {
            let _ = kept.hangup().await;
        }
    }

    /// Give up: what Alice's phone saw and the node's logs.
    async fn fail(&self, why: &str) -> ! {
        let mut seen = Vec::new();
        if let Some(alice) = self.phones.get(&Ph::User(Alice)) {
            while let Some(ev) = alice.next_event(Duration::from_millis(50)).await {
                seen.push(ev);
            }
        }
        let logs = match self.stack.service.lock().await.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        panic!("{}: {why}\nAlice saw: {seen:#?}\n{logs}", self.row);
    }
}

fn find_me(steps: &[&[To]]) -> fcp_core::FindMe {
    let steps: Vec<_> = steps
        .iter()
        .map(|targets| {
            serde_json::json!({
                "targets": targets.iter().map(|t| t.find_me()).collect::<Vec<_>>(),
                "ring_secs": STEP_SECS,
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({ "enabled": true, "steps": steps })).expect("find-me")
}

async fn run(name: &'static str, row: &Row) {
    // A phone that answers or rings first cannot also be one that never
    // rings: such a row would pass whatever the node did.
    for ph in row.then.silent {
        let rings = row.then.first.contains(ph)
            || matches!(row.then.outcome, Outcome::AnsweredBy(by) if by == *ph);
        assert!(!rings, "{name}: {ph:?} is both silent and rung");
    }
    let Some(mut world) = World::build(name, row.given).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    eprintln!("{name}: {}", row.why);
    let call = world.when(row.when).await;
    world.then(row.when, row.then, call).await;
    world.stack.finish().await;
}
