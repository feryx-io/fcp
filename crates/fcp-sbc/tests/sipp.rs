//! The SBC against SIPp (`docs/DEEP_TESTING.md` D2): a carrier trunk and
//! the core behind it are both SIPp, so nothing of siphon-rs is on either
//! side of the SBC. Calls answered, refused, cancelled, re-INVITEd and
//! UPDATEd across it; requests for no dialog; broken messages; and a flood
//! from one carrier address. The SBC runs without a database: trunks
//! from its configuration, the carrier known by its source address.
//! Skipped without SIPp unless `FCP_SIPP_TESTS=required`. The scenarios
//! are the call manager's (`fcp-call-manager/tests/sipp/`).

use fcp_testkit::sipp::{Sipp, SippOutcome, SippRun};
use fcp_testkit::smoke::{free_port, http_get, loopback};
use fcp_testkit::Service;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-sbc");
/// The carrier's address, and a second carrier's.
const CARRIER: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
const OTHER_CARRIER: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 4);
/// The core's address (a SIPp callee standing in for a call manager).
const CORE: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 3);
/// The number the carrier calls.
const NUMBER: &str = "+15550001000";

fn scenario(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fcp-call-manager/tests/sipp")
        .join(format!("{name}.xml"))
}

/// One trunk, its OPTIONS probes off: a SIPp peer answers them only when
/// its scenario says so.
fn trunk(name: &str, direction: &str, host: SocketAddr, source: Ipv4Addr) -> String {
    format!(
        r#"
[[core.trunks]]
name = "{name}"
enabled = true
direction = "{direction}"
hosts = ["{host}"]
inbound_sources = ["{source}/32"]
priority = 1
weight = 100
register = false
max_concurrent_calls = 0

[core.trunks.failover]
max_failures = 5
recovery_interval_secs = 30
health_check_enabled = false
health_check_interval_secs = 30
"#
    )
}

/// An SBC between two carriers and a core.
struct Sbc {
    service: tokio::sync::Mutex<Option<Service>>,
    sip: SocketAddr,
    http: u16,
}

impl Sbc {
    /// An SBC sending the carriers' calls to `core`. `rate_limit` keeps
    /// the per-source request limits on, as shipped.
    async fn start(core: SocketAddr, rate_limit: bool) -> Self {
        let sip = free_port();
        let http = free_port();
        let rtp = fcp_testkit::media::allocate_rtp_range(100);
        let unused = SocketAddr::from((CARRIER, 9));
        let config = format!(
            r#"[service]
node_id = "sipp-sbc"
drain_wait_secs = 0

[core]
external_domain = "sbc.test"

[core.nat]
rtp_port_start = {rtp}
rtp_port_end = {rtp_end}

[core.rate_limit]
enabled = {rate_limit}
{carrier}{other}{core}"#,
            rtp_end = rtp + 100,
            carrier = trunk("carrier", "downstream", unused, CARRIER),
            other = trunk("other-carrier", "downstream", unused, OTHER_CARRIER),
            core = trunk("core", "upstream", core, CORE),
        );
        let bind = format!("127.0.0.1:{sip}");
        let health = format!("127.0.0.1:{http}");
        let service = Service::start_with(
            "fcp-sbc",
            BIN,
            &config,
            &loopback(http, "/health"),
            &[
                ("FCP_SBC_BIND", bind.as_str()),
                ("FCP_SBC_HEALTH_BIND", health.as_str()),
            ],
        )
        .await;
        Self {
            service: tokio::sync::Mutex::new(Some(service)),
            sip: SocketAddr::from(([127, 0, 0, 1], sip)),
            http,
        }
    }

    /// Panic, with SIPp's errors and messages and the SBC's log, unless
    /// every call of the run passed.
    async fn passed(&self, out: &SippOutcome, what: &str) {
        if out.code == Some(0) {
            return;
        }
        let logs = match self.service.lock().await.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        eprintln!("--- sbc ---\n{logs}");
        out.assert_passed(what);
    }

    /// The SBC's calls not yet over.
    async fn live_calls(&self) -> Vec<serde_json::Value> {
        let (code, body) = http_get(&loopback(self.http, "/admin/calls"))
            .await
            .expect("GET /admin/calls");
        assert_eq!(code, 200, "{body}");
        serde_json::from_str::<Vec<serde_json::Value>>(&body)
            .unwrap_or_else(|e| panic!("/admin/calls: {e}: {body}"))
    }

    /// Once no call is up (or panic after 5 s).
    async fn no_calls_up(&self, what: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let live = self.live_calls().await;
            if live.is_empty() {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what}: calls still up: {live:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// `caller` run from `from` at the SBC, to [`NUMBER`].
    fn caller(&self, sipp: &Sipp, from: Ipv4Addr, caller: &str, args: &[&str]) -> SippRun {
        let domain = self.sip.to_string();
        sipp.start_at(
            from,
            &scenario(caller),
            Some(self.sip),
            &[("user", "carrier"), ("domain", &domain), ("callee", NUMBER)],
            args,
        )
    }

    async fn passed_both(&self, uac: &SippOutcome, uas: &SippOutcome, what: &str) {
        if uac.code != Some(0) || uas.code != Some(0) {
            eprintln!(
                "--- core ({:?}) errors ---\n{}\n--- core messages ---\n{}",
                uas.code, uas.errors, uas.messages
            );
        }
        self.passed(uac, &format!("{what}: carrier")).await;
        self.passed(uas, &format!("{what}: core")).await;
    }
}

/// A call from the carrier through the SBC to the core: `caller` against
/// `callee`, both passing, and nothing left up.
async fn call(caller: &str, callee: &str) -> Option<(SippOutcome, SippOutcome)> {
    let sipp = Sipp::find()?;
    let core = sipp.start_at(CORE, &scenario(callee), None, &[], &[]);
    let sbc = Sbc::start(core.addr(), false).await;
    let uac = sbc
        .caller(&sipp, CARRIER, caller, &[])
        .wait(Duration::from_secs(40))
        .await;
    let uas = core.wait(Duration::from_secs(10)).await;
    sbc.passed_both(&uac, &uas, caller).await;
    sbc.no_calls_up(caller).await;
    Some((uac, uas))
}

#[tokio::test]
async fn a_carrier_call_is_answered_and_hung_up() {
    let Some((_, uas)) = call("trunk_answered", "uas_answer").await else {
        return;
    };
    // A new dialog toward the core: the carrier's Call-ID stays outside.
    assert!(!uas.messages.contains("SIPpTag00"), "{}", uas.messages);
}

#[tokio::test]
async fn a_carrier_call_is_refused_busy() {
    call("trunk_busy", "uas_busy").await;
}

#[tokio::test]
async fn a_carrier_cancels_a_ringing_call() {
    call("trunk_cancel", "uas_ring").await;
}

#[tokio::test]
async fn a_carrier_reinvites_an_answered_call() {
    call("trunk_reinvite", "uas_reinvite").await;
}

#[tokio::test]
async fn a_carrier_updates_an_answered_call() {
    if let Some((_, uas)) = call("trunk_update", "uas_until_bye").await {
        assert!(uas.received_request("UPDATE"), "{}", uas.messages);
    }
}

/// Requests inside dialogs that do not exist, from the carrier: 481 or
/// refused, nothing reaching the core, nothing left up.
#[tokio::test]
async fn requests_for_no_dialog_are_refused() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    let core = sipp.start_at(CORE, &scenario("uas_answer"), None, &[], &[]);
    let sbc = Sbc::start(core.addr(), false).await;
    let out = sbc
        .caller(&sipp, CARRIER, "strays", &[])
        .wait(Duration::from_secs(40))
        .await;
    sbc.passed(&out, "strays").await;
    let core = core.wait(Duration::from_millis(500)).await;
    assert!(!core.received_request("INVITE"), "{}", core.messages);
    sbc.no_calls_up("strays").await;
}

/// Messages no parser should accept, sent at the SBC: it answers the
/// OPTIONS after them, and its health server still does too.
#[tokio::test]
async fn messages_no_parser_should_accept_are_survived() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    let core = sipp.start_at(CORE, &scenario("uas_answer"), None, &[], &[]);
    let sbc = Sbc::start(core.addr(), false).await;
    let out = sbc
        .caller(&sipp, CARRIER, "junk", &[])
        .wait(Duration::from_secs(40))
        .await;
    sbc.passed(&out, "junk").await;
    let (code, body) = http_get(&loopback(sbc.http, "/health"))
        .await
        .expect("health");
    assert_eq!(code, 200, "{body}");
    sbc.no_calls_up("junk").await;
}

/// How many of a run's requests were answered `code`.
fn answered(out: &SippOutcome, code: u16) -> usize {
    out.messages
        .split("-----------------------------------------------")
        .filter(|m| m.contains("message received") && m.contains(&format!("SIP/2.0 {code} ")))
        .count()
}

/// An INVITE flood from one carrier address, the SBC's request limits on
/// as shipped: the flood is refused 429, while a call from the other
/// carrier's address still reaches the core and gets its answer.
#[tokio::test]
async fn a_flood_from_one_carrier_costs_that_carrier() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    // The core refuses everything busy, so a call that gets through ends.
    let core = sipp.start_at(
        CORE,
        &scenario("uas_busy"),
        None,
        &[],
        &["-m", "1000", "-timeout", "20s"],
    );
    let sbc = Sbc::start(core.addr(), true).await;
    let flood = sbc.caller(
        &sipp,
        CARRIER,
        "flood_invite",
        &["-r", "200", "-m", "400", "-timeout", "20s"],
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let other = sbc
        .caller(&sipp, OTHER_CARRIER, "trunk_busy", &[])
        .wait(Duration::from_secs(40))
        .await;
    sbc.passed(&other, "the other carrier's call").await;
    let flood = flood.wait(Duration::from_secs(30)).await;
    let (refused, busy) = (answered(&flood, 429), answered(&flood, 486));
    eprintln!("INVITE flood at the SBC: {refused} refused 429, {busy} reached the core");
    assert!(refused > 100, "{refused} of 400 refused: {}", flood.errors);
    assert!(busy > 0, "the limit refused every INVITE");
    let _ = core.wait(Duration::from_secs(1)).await;
    sbc.no_calls_up("flood").await;
}

/// OPTIONS at 400 a second, 1,200 of them, from `from`: how many were
/// answered.
async fn options_at_400(sipp: &Sipp, sbc: &Sbc, from: Ipv4Addr) -> usize {
    let domain = sbc.sip.to_string();
    let out = sipp
        .start_at(
            from,
            &scenario("flood_options"),
            Some(sbc.sip),
            &[("user", "probe"), ("domain", &domain)],
            &["-r", "400", "-m", "1200", "-timeout", "20s"],
        )
        .wait(Duration::from_secs(30))
        .await;
    answered(&out, 200)
}

/// A carrier carries many parties' traffic, so the address its trunk
/// takes calls from is exempt from the per-source message limit with no
/// configuration: 1,200 OPTIONS at 400 a second all answered. An address
/// no trunk names loses what is over 200 a second.
#[tokio::test]
async fn a_trunk_source_is_not_limited_per_source() {
    let Some(sipp) = Sipp::find() else {
        return;
    };
    let core = sipp.start_at(CORE, &scenario("uas_answer"), None, &[], &[]);
    let sbc = Sbc::start(core.addr(), false).await;
    let carrier = options_at_400(&sipp, &sbc, CARRIER).await;
    let stranger = options_at_400(&sipp, &sbc, Ipv4Addr::new(127, 0, 0, 5)).await;
    eprintln!("OPTIONS at 400/s: carrier {carrier} of 1200 answered, stranger {stranger}");
    assert!(
        carrier >= 1190,
        "a trunk's source was limited: {carrier} of 1200"
    );
    assert!(
        stranger < 1000,
        "the limit no longer applies: {stranger} of 1200"
    );
}
