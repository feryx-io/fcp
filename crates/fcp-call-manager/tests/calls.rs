//! Calls through the call manager over real SIP and RTP
//! (`docs/TEST_FLOOR.md`, T1 and T2): phones register, call each other,
//! hear each other, hold, send digits, transfer and hang up, against the
//! binary on loopback with a throwaway database. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Phone, PhoneEvent, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(5);

/// A running call manager on a fresh database, with what a test needs
/// to reach it.
struct Stack {
    service: tokio::sync::Mutex<Option<Service>>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database to test against.
    async fn start() -> Option<Self> {
        Self::start_with("", 40).await
    }

    /// As [`Stack::start`], with `extra` TOML appended to the config and
    /// `rtp_ports` in the media range (a queue wants at least 100).
    async fn start_with(extra: &str, rtp_ports: u16) -> Option<Self> {
        Self::start_configured("", extra, rtp_ports).await
    }

    /// As [`Stack::start_with`], with `media` added to the `[media]`
    /// section. `{dir}` in either names the stack's own directory.
    async fn start_configured(media: &str, extra: &str, rtp_ports: u16) -> Option<Self> {
        Self::start_registrar("", media, extra, rtp_ports).await
    }

    /// As [`Stack::start_configured`], with `registrar` added to the
    /// `[registrar]` section — which cannot go in `extra`, since a second
    /// `[registrar]` table is not valid TOML.
    async fn start_registrar(
        registrar: &str,
        media: &str,
        extra: &str,
        rtp_ports: u16,
    ) -> Option<Self> {
        Self::start_node(
            r#"local_ip = "127.0.0.1""#,
            registrar,
            media,
            extra,
            rtp_ports,
        )
        .await
    }

    /// A node that advertises `public_ip` in Contact and SDP, as one on a
    /// cloud VM does when it is told only its public address.
    async fn start_advertising(public_ip: &str) -> Option<Self> {
        Self::start_node("", "", &format!(r#"public_ip = "{public_ip}""#), "", 60).await
    }

    async fn start_node(
        local_ip: &str,
        registrar: &str,
        media: &str,
        extra: &str,
        rtp_ports: u16,
    ) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let config = format!(
            r#"
[service]
node_id = "e2e-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
{local_ip}
# Every phone here is 127.0.0.1, one source to the per-source limit: forty
# calls back to back went over it and lost a 200 (sipp.rs tests the limit).
max_messages_per_source = 0

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

[b2bua.forking]
timeout = 3

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"
{media}

[logging]
level = "info"
format = "compact"

{extra}
"#,
            db = db.url(),
            rtp_end = rtp_start + rtp_ports,
            dir = dir.path().display(),
            registrar = registrar,
            media = media.replace("{dir}", &dir.path().display().to_string()),
            extra = extra.replace("{dir}", &dir.path().display().to_string()),
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            service: tokio::sync::Mutex::new(Some(service)),
            db: Some(db),
            seed,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
            _dir: dir,
        })
    }

    async fn phone(&self, user: &SeededUser) -> Phone {
        Phone::new(&user.username, &user.password, REALM, self.sip)
            .await
            .expect("phone binds")
    }

    /// The records at `/cdrs`, once at least `n` are there.
    async fn cdrs(&self, n: usize, within: Duration) -> Vec<serde_json::Value> {
        self.cdrs_at("/cdrs", n, within).await
    }

    /// The records at `path` (`/cdrs?…`), once at least `n` are there.
    async fn cdrs_at(&self, path: &str, n: usize, within: Duration) -> Vec<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let body = self.get(path).await;
            let cdrs = body["cdrs"].as_array().cloned().unwrap_or_default();
            if cdrs.len() >= n {
                return cdrs;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} of {n} CDRs after {within:?}: {body}",
                cdrs.len()
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// The stack's own directory: recordings go under `recordings/`.
    fn dir(&self) -> &std::path::Path {
        self._dir.path()
    }

    /// POST `body` to `path`: the status and the JSON answer.
    async fn post(&self, path: &str, body: &str) -> (u16, serde_json::Value) {
        let (code, text) = fcp_testkit::smoke::http_post(&loopback(self.http, path), body)
            .await
            .unwrap_or_else(|e| panic!("POST {path}: {e}"));
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
        (code, json)
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(code, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
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

    /// Stop the service (a clean exit is asserted) and drop the database;
    /// the service's log.
    async fn finish(mut self) -> String {
        let log = match self.service.lock().await.take() {
            Some(service) => service.stop().await,
            None => String::new(),
        };
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
        log
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn registration_is_digest_authenticated_and_listed() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;

    // The right password registers; the registration is listed.
    let phone = stack.phone(&alice).await;
    assert_eq!(phone.register().await.unwrap(), 200);
    let regs = stack.get("/registrations").await;
    assert_eq!(regs["count"], 1, "{regs}");
    assert!(
        regs["registrations"][0]["aor"]
            .as_str()
            .unwrap_or("")
            .contains("alice@"),
        "{regs}"
    );

    // The wrong password does not, nor does a user that does not exist.
    let wrong = Phone::new("alice", "not-her-password", REALM, stack.sip)
        .await
        .unwrap();
    let code = wrong.register().await.unwrap();
    assert!(matches!(code, 401 | 403), "wrong password answered {code}");
    let nobody = Phone::new("nobody", "x", REALM, stack.sip).await.unwrap();
    let code = nobody.register().await.unwrap();
    assert!(!(200..300).contains(&code), "unknown user answered {code}");

    stack.finish().await;
}

/// The routing engine's users are the platform's, and a caller that is
/// not one of the tenant's users is refused with 403, never a 500
/// (`docs/PRODUCTION_STACK.md` phase 5). A phone's own INVITE is
/// digest-authenticated before the engine sees it, so the caller with no
/// user arrives the one way it can: a trusted edge naming one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_routing_engine_refuses_a_caller_who_is_not_a_user() {
    let rules = tempfile::tempdir().expect("tempdir");
    let rules_file = rules.path().join("routing.yaml");
    std::fs::write(
        &rules_file,
        r#"users: []
policies:
  - name: allow-tenant-users
    tenant_id: "00000000-0000-0000-0000-000000000000"
    priority: 100
    enabled: true
    applies_to:
      all_users: true
    destination_match: null
    time_windows: []
    source_ip_ranges: []
    action: Allow
    reason: "The tenant's users may call"
"#,
    )
    .expect("rules file");
    let extra = format!(
        "[routing]\nrules_file = \"{}\"\n\n[e911]\ntrusted_edges = [\"127.0.0.1\"]\n",
        rules_file.display()
    );
    let Some(stack) = Stack::start_with(&extra, 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    bob_phone.auto_answer(true);
    let tenant = stack.seed.tenant_id.to_string();

    // A registered phone's call is its user's, found in the database: it
    // passes the file's policy and rings Bob.
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    let call = alice_phone.call("1002").await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(
                &format!("Alice's own call was not answered: {e}"),
                &alice_phone,
            )
            .await;
    }
    call.hangup().await.unwrap();

    // The edge names a user the tenant does not have: 403 with the reason.
    let edge = Phone::new("edge", "x", REALM, stack.sip).await.unwrap();
    let call = edge
        .call_with_headers(
            "1002",
            &[
                (fcp_core::CALLER_TENANT_HEADER, tenant.as_str()),
                (fcp_core::CALLER_USER_HEADER, "nobody"),
            ],
        )
        .await
        .unwrap();
    match call.failed(SOON).await {
        Some(403) => {}
        other => {
            stack
                .fail(&format!("a stranger's call got {other:?}, not 403"), &edge)
                .await
        }
    }

    // The edge names Alice: the call is hers, and passes.
    let call = edge
        .call_with_headers(
            "1002",
            &[
                (fcp_core::CALLER_TENANT_HEADER, tenant.as_str()),
                (fcp_core::CALLER_USER_HEADER, "alice"),
            ],
        )
        .await
        .unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(
                &format!("the call the edge placed as Alice failed: {e}"),
                &edge,
            )
            .await;
    }
    call.hangup().await.unwrap();

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_invite_is_admitted_by_its_credentials_not_its_address() {
    // `[registrar] auth_invites` (`docs/EDGE_SECURITY.md` §2.1). Every
    // phone here shares one address — loopback — which is exactly the
    // shape the old gate could not tell apart: source-based admission let
    // all of them place calls as each other. With the flag on, the digest
    // decides, so the phone that cannot answer a challenge cannot call
    // while its neighbour at the same address can.
    let Some(stack) = Stack::start_registrar("auth_invites = true", "", "", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    bob_phone.auto_answer(true);

    // A phone at the very same address that cannot prove it is Alice.
    let impostor = Phone::new(&alice.username, "not-alice-pw", REALM, stack.sip)
        .await
        .unwrap();
    impostor.call(&bob.extension).await.unwrap();
    let refused = impostor
        .expect(SOON, |e| matches!(e, PhoneEvent::Failed { .. }))
        .await;
    match refused {
        Some(PhoneEvent::Failed { code, .. }) => assert_eq!(
            code, 401,
            "an INVITE without usable credentials is challenged, not refused outright"
        ),
        other => {
            let why = format!("the impostor's call was not refused: {other:?}");
            stack.fail(&why, &impostor).await;
        }
    }
    assert!(
        bob_phone
            .incoming(Duration::from_millis(500))
            .await
            .is_none(),
        "Bob's phone rang for a caller who proved nothing"
    );

    // Alice herself, from the same address, is let through.
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    let call = alice_phone.call(&bob.extension).await.unwrap();
    if call.answered(SOON).await.is_err() {
        let why = format!("Alice's own call did not connect: {:?}", call.phase());
        stack.fail(&why, &alice_phone).await;
    }
    call.hangup().await.unwrap();

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_does_not_choose_what_its_record_points_at() {
    // `docs/EDGE_SECURITY.md` §2.3. `X-FCP-Transferred-From` names the
    // call a transferred leg came from, and the B2BUA filed it from
    // whoever sent it. Records are rated, billed, rolled up per tenant
    // and exported for compliance, so a caller choosing the id chooses
    // whose call their leg is correlated with. A transfer this node
    // actually relayed still correlates — that is
    // `a_blind_transfer_moves_the_caller_to_a_third_phone` — but an
    // invented id names nothing and is taken away.
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let invented = "call-11111111-1111-1111-1111-111111111111";
    let call = alice_phone
        .call_with_headers(
            &bob.extension,
            &[
                ("X-FCP-Transferred-From", invented),
                ("X-FCP-Call-Id", invented),
                ("P-Asserted-Identity", "<sip:+15125550100@carrier.example>"),
                ("Alert-Info", "<http://x/>;info=alert-autoanswer;delay=0"),
                ("Call-Info", "<sip:>;answer-after=0"),
                ("X-FCP-Intercom-Mode", "oneway"),
            ],
        )
        .await
        .unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let why = format!("Bob's phone did not ring; the call is {:?}", call.phase());
        stack.fail(&why, &alice_phone).await;
    };

    // Nothing the caller claimed reaches Bob's phone.
    for claimed in [
        "Alert-Info",
        "Call-Info",
        "X-FCP-Intercom-Mode",
        "X-FCP-Transferred-From",
    ] {
        assert_eq!(
            incoming.header(claimed),
            None,
            "Bob's phone was told {claimed} by a caller with no standing to say it"
        );
    }
    // The call's id Bob's phone is told is FCP's (`docs/PUSH.md` §2.5).
    let named = incoming.header("X-FCP-Call-Id");
    assert!(
        named.as_deref().is_some_and(|id| !id.contains("1111")),
        "Bob's phone should be told FCP's id for the call, not the caller's: {named:?}"
    );
    assert_ne!(
        incoming.header("P-Asserted-Identity").as_deref(),
        Some("<sip:+15125550100@carrier.example>"),
        "a caller is not the network that asserts who is calling"
    );

    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(bob_call.hangup().await.unwrap(), 200);

    // And the record points at nothing, rather than at a call of the
    // caller's choosing.
    let records = stack.cdrs(1, SOON).await;
    let record = records
        .iter()
        .find(|r| r["to_uri"].as_str().unwrap_or("").contains(":1002@"))
        .unwrap_or_else(|| panic!("no record of the call to Bob: {records:#?}"));
    assert!(
        record["correlation_id"].is_null(),
        "the caller filed its record against a call it named: {record}"
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_source_cannot_spend_the_whole_node() {
    // `docs/EDGE_SECURITY.md` §2.4. `[admission] max_cps` is a node-wide
    // budget, so one source could spend all of it, and the handler's own
    // limiters are per-dialog — they bound one caller's mischief and
    // nothing about the node. A per-source rate is what the SBC had and
    // the call manager, the edge of every single-binary deployment, did
    // not.
    // Without INVITE challenges: each would cost a second INVITE against
    // the same per-source budget (`fcp_sip::edge`), and eight calls fired
    // at once would spend it all before any retry, which is the limit
    // working, not this test's question.
    let Some(stack) = Stack::start_registrar(
        "auth_invites = false",
        "",
        "[security]\nmax_invite_cps_per_source = 2\n",
        40,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    bob_phone.auto_answer(true);

    // More calls in one second than the node allows from one address.
    let mut calls = Vec::new();
    for _ in 0..8 {
        calls.push(alice_phone.call(&bob.extension).await.unwrap());
    }

    // The ones over the limit are refused, and told when to come back.
    let mut refused = 0;
    while let Some(event) = alice_phone.next_event(SOON).await {
        if let PhoneEvent::Failed { code: 503, .. } = event {
            refused += 1;
        }
    }
    assert!(
        refused > 0,
        "eight calls in one second from one address, none refused"
    );
    assert!(
        refused < calls.len(),
        "the limit refused every call, not just the ones over it"
    );

    for call in calls {
        let _ = call.hangup().await;
    }
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_the_node_denies_is_told_nothing() {
    // `docs/EDGE_SECURITY.md` §2.4: the call manager had no address list
    // at all. A denied peer is dropped rather than answered — replying
    // confirms the port is live and makes the node a reflector — so the
    // phone here waits and gets nothing, which is the point.
    let acl = r#"[security]
acl = { default_action = "Allow", rules = [{ cidr = "127.0.0.1/32", action = "Deny" }] }
"#;
    let Some(stack) = Stack::start_with(acl, 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let alice_phone = stack.phone(&alice).await;

    assert!(
        alice_phone.register().await.is_err(),
        "a denied address was answered"
    );

    // The node is otherwise alive: it is refusing this address, not ill.
    let health = stack.get("/health").await;
    assert_eq!(health["status"], "healthy", "{health}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_setup_says_how_many_registration_reads_it_costs() {
    // `docs/MULTI_REGION_FOLLOWUPS.md` §2.1, phase 1. Registrations are
    // read from the primary, so in a deployment whose primary is in
    // another region each of these is a round trip across it. The survey
    // estimated the count; this measures it.
    //
    // A count, not a timing: a loaded CI box says nothing about latency.
    //
    // With the cache off: this is the cost without it, which phase 2's
    // tests (`tests/registrations.rs`) measure the cache against.
    let Some(stack) = Stack::start_registrar("cache_ttl_secs = 0", "", "", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    bob_phone.auto_answer(true);

    let reads = |body: &serde_json::Value| -> (u64, u64, u64) {
        let r = &body["registration_reads"];
        (
            r["direct"].as_u64().unwrap_or_default(),
            r["alias"].as_u64().unwrap_or_default(),
            r["total"].as_u64().unwrap_or_default(),
        )
    };

    let before = reads(&stack.get("/status").await);
    let call = alice_phone.call(&bob.extension).await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    let after = reads(&stack.get("/status").await);

    let direct = after.0 - before.0;
    let alias = after.1 - before.1;
    let total = after.2 - before.2;

    // What the survey claimed: at least the caller's bindings for
    // admission and the callee's for routing. If this ever drops to zero
    // the counter has come unwired, and if it climbs the call path has
    // grown a lookup nobody noticed.
    assert!(
        (2..=6).contains(&direct),
        "a two-party call made {direct} direct registration reads \
         (alias {alias}, total {total}); §2.1 expects about two"
    );
    // No domain aliases are configured here, so none of them are alias
    // reads — that multiplication is a deployment's to pay.
    assert_eq!(
        alias, 0,
        "no aliases are configured, so no lookup should be an alias read"
    );
    // Housekeeping (REGISTER handling, expiry) is counted apart, so the
    // total is never less than the call-path reads.
    assert!(
        total >= direct + alias,
        "total {total} is less than the call-path reads ({direct} + {alias})"
    );
    eprintln!(
        "PHASE1 registration reads for one two-party call: \
         direct={direct} alias={alias} total={total}"
    );

    // And the same counts are on /metrics, which is what a deployment
    // actually graphs.
    let (code, body) = fcp_testkit::smoke::http_request(
        "GET",
        &fcp_testkit::smoke::loopback(stack.http, "/metrics"),
        &[],
        None,
    )
    .await
    .expect("metrics");
    assert_eq!(code, 200);
    assert!(
        body.contains("fcp_registration_reads_total{") && body.contains("reason=\"direct\""),
        "the reads are not on /metrics"
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn domain_aliases_multiply_every_registration_read() {
    // The other half of `docs/MULTI_REGION_FOLLOWUPS.md` §2.1: each alias
    // is its own round trip, so a deployment with aliases pays the §2.1
    // cost several times over. Estimated in the survey; measured here.
    // The cache off, as above.
    let aliases = r#"domain_aliases = ["e2e.local", "alias-one.local", "alias-two.local"]
cache_ttl_secs = 0"#;
    let Some(stack) = Stack::start_registrar(aliases, "", "", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    bob_phone.auto_answer(true);

    let counts = |body: &serde_json::Value| -> (u64, u64) {
        let r = &body["registration_reads"];
        (
            r["direct"].as_u64().unwrap_or_default(),
            r["alias"].as_u64().unwrap_or_default(),
        )
    };

    let before = counts(&stack.get("/status").await);
    let call = alice_phone.call(&bob.extension).await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    let after = counts(&stack.get("/status").await);

    let direct = after.0 - before.0;
    let alias = after.1 - before.1;
    eprintln!(
        "PHASE1 with two aliases: direct={direct} alias={alias} \
         (total {})",
        direct + alias
    );

    // Two aliases beside the AOR's own domain: each direct lookup is
    // followed by a lookup at each of the others.
    assert!(
        alias > 0,
        "aliases are configured but no lookup was counted as an alias read"
    );
    assert!(
        alias >= direct,
        "with two aliases each lookup should cost at least one more \
         (direct {direct}, alias {alias})"
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_is_answered_heard_both_ways_and_recorded() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    // Alice dials Bob's extension; Bob rings, then answers.
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let why = format!(
            "Bob's phone did not ring; Alice's call is {:?}",
            alice_call.phase()
        );
        stack.fail(&why, &alice_phone).await;
    };
    assert!(incoming.from.contains("alice"), "{}", incoming.from);
    assert!(
        alice_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Ringing { code: 180, .. }))
            .await
            .is_some(),
        "Alice hears ringing"
    );
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Audio both ways: a tone from each side heard at the other.
    alice_call.media().play_tone(440.0, 1500);
    assert!(
        bob_call.media().heard(440, SOON).await,
        "Bob hears Alice's 440 Hz (received {} packets)",
        bob_call.media().packets_received()
    );
    bob_call.media().play_tone(880.0, 1500);
    assert!(
        alice_call.media().heard(880, SOON).await,
        "Alice hears Bob's 880 Hz (received {} packets)",
        alice_call.media().packets_received()
    );

    // The call is visible while it is up.
    let calls = stack.get("/calls").await;
    assert!(
        calls["calls"].as_array().is_some_and(|c| !c.is_empty()),
        "{calls}"
    );

    // Alice hangs up; Bob's phone gets the BYE.
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Bob's phone gets BYE"
    );
    assert!(bob_call.ended(SOON).await);

    // And the call's record says what happened (docs/CDR_PIPELINE.md §5):
    // answered, hung up by the caller, the codec, media both ways, the node.
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["kind"], "call", "{cdr}");
    assert!(
        cdr["cdr_id"].as_str().is_some_and(|id| !id.is_empty()),
        "{cdr}"
    );
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert_eq!(cdr["hangup_cause"], "NormalClearing", "{cdr}");
    assert_eq!(cdr["hangup_party"], "Caller", "{cdr}");
    assert!(cdr["call_duration_ms"].as_u64().unwrap_or(0) > 0, "{cdr}");
    assert_eq!(cdr["codec_a_leg"], "PCMU", "{cdr}");
    assert_eq!(cdr["node_id"], "e2e-call-manager", "{cdr}");
    assert!(
        cdr["rtp_packets_received"].as_u64().unwrap_or(0) > 0
            && cdr["rtp_packets_sent"].as_u64().unwrap_or(0) > 0,
        "the anchored media's counters are on the record: {cdr}"
    );
    // The pipeline's status: the spool drained (the cursor commits just
    // after the record lands), one record stored here.
    let deadline = tokio::time::Instant::now() + SOON;
    let status = loop {
        let status = stack.get("/cdr/status").await;
        if status["spool"]["pending_bytes"] == 0 {
            break status;
        }
        assert!(tokio::time::Instant::now() < deadline, "{status}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(status["processor"]["stored"], 1, "{status}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_callee_can_hang_up() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let bob_call = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .answer()
        .await
        .unwrap();
    alice_call.answered(SOON).await.unwrap();

    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    assert!(
        alice_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Alice's phone gets BYE"
    );
    assert!(alice_call.ended(SOON).await);

    // The record knows it was the callee who hung up.
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert_eq!(cdr["hangup_party"], "Callee", "{cdr}");

    stack.finish().await;
}

/// #335: a node on a cloud VM whose public address is NAT at the edge
/// advertises an address this host does not reach (TEST-NET-3 here). A
/// phone on the same host was sent that address in Contact and SDP, so its
/// media and its BYE were lost; it is now given the node's local address.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_phone_on_the_nodes_host_is_given_an_address_it_reaches() {
    if std::env::var("FCP_TEST_DATABASE_URL").is_err() {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping");
        return;
    }
    let stack = Stack::start_advertising("203.0.113.7")
        .await
        .expect("the stack starts");
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    alice_call.media().play_tone(440.0, 1500);
    assert!(
        bob_call.media().heard(440, SOON).await,
        "Bob hears Alice (received {} packets)",
        bob_call.media().packets_received()
    );
    bob_call.media().play_tone(880.0, 1500);
    assert!(
        alice_call.media().heard(880, SOON).await,
        "Alice hears Bob (received {} packets)",
        alice_call.media().packets_received()
    );

    // In-dialog requests go to the node's Contact.
    assert_eq!(
        alice_call.hangup().await.unwrap(),
        200,
        "Alice's BYE is answered"
    );
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Bob's phone gets BYE"
    );
    let log = stack.finish().await;
    assert!(
        log.contains("Services on this host are told this host's own address"),
        "{log}"
    );
}

/// A call's SIP trace is what crossed the wire: every message the node sent
/// and received, between two addresses that are each `ip:port`, so the
/// admin's ladder has one column per party. Before, the node recorded at
/// places of its own: a BYE it rebuilt with a branch no response matched,
/// responses with no destination, the B-leg's INVITE "to" a URI, and none
/// of the ACKs or replies to a BYE it sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_s_trace_is_what_crossed_the_wire() {
    use fcp_core::SipTraceStore;
    let stack = stack_or_skip!();
    let (_alice, alice_phone, bob, bob_phone) = two_phones(&stack).await;
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let bob_call = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .answer()
        .await
        .unwrap();
    alice_call.answered(SOON).await.unwrap();
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    assert!(alice_call.ended(SOON).await);
    let cdr = stack.cdrs(1, SOON).await.remove(0);

    // The trace is written in batches, filed under the node's own id for
    // the call, which the caller's Call-ID leads to.
    let store = fcp_storage_postgres::sip_trace::PostgresSipTraceStore::new(
        stack.seed.storage().pool().clone(),
    );
    let sip_call_id = cdr["sip_call_id"].as_str().unwrap().to_string();
    let deadline = tokio::time::Instant::now() + SOON;
    let trace = loop {
        let a_leg = store
            .get_sip_traces_by_sip_call_id(&sip_call_id)
            .await
            .unwrap();
        let trace = match a_leg.first() {
            Some(first) => store.get_sip_traces(&first.call_id).await.unwrap(),
            None => Vec::new(),
        };
        let ended = trace
            .iter()
            .filter(|m| m.cseq.ends_with(" BYE") && m.method_or_status == "200")
            .count();
        if ended >= 2 || tokio::time::Instant::now() > deadline {
            break trace;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let ladder: Vec<String> = trace
        .iter()
        .map(|m| {
            format!(
                "{} {} {} -> {} [{}]",
                m.direction, m.method_or_status, m.source_addr, m.dest_addr, m.cseq
            )
        })
        .collect();
    let ladder = ladder.join("\n");

    // Every message is between two addresses, and the node is always the same one.
    let node = format!("{}", stack.sip);
    for m in &trace {
        for addr in [&m.source_addr, &m.dest_addr] {
            assert!(addr.parse::<SocketAddr>().is_ok(), "{addr:?} in\n{ladder}");
        }
        let ours = if m.direction == "inbound" {
            &m.dest_addr
        } else {
            &m.source_addr
        };
        assert_eq!(ours, &node, "\n{ladder}");
    }
    // Both legs, whole: the caller's INVITE (challenged, then sent again)
    // and its ACKs, the B-leg's INVITE, each answer, the B-leg's ACK, the
    // BYE and its 200, each once.
    let count = |direction: &str, what: &str, cseq: &str| {
        trace
            .iter()
            .filter(|m| {
                m.direction == direction && m.method_or_status == what && m.cseq.ends_with(cseq)
            })
            .count()
    };
    assert_eq!(count("inbound", "INVITE", "INVITE"), 2, "\n{ladder}");
    assert_eq!(count("outbound", "401", "INVITE"), 1, "\n{ladder}");
    assert_eq!(count("outbound", "INVITE", "INVITE"), 1, "\n{ladder}");
    assert_eq!(count("inbound", "200", "INVITE"), 1, "\n{ladder}");
    assert_eq!(count("outbound", "200", "INVITE"), 1, "\n{ladder}");
    assert_eq!(count("inbound", "ACK", "ACK"), 2, "\n{ladder}");
    assert_eq!(
        count("outbound", "ACK", "ACK"),
        1,
        "the B-leg's ACK\n{ladder}"
    );
    assert_eq!(count("inbound", "BYE", "BYE"), 1, "\n{ladder}");
    assert_eq!(
        count("outbound", "200", "BYE"),
        1,
        "the 200 to Bob's BYE\n{ladder}"
    );
    assert_eq!(count("outbound", "BYE", "BYE"), 1, "\n{ladder}");
    assert_eq!(count("inbound", "200", "BYE"), 1, "\n{ladder}");
    // The BYE the node sent is the one its 200 answers: the same branch.
    let branch = |raw: &str| {
        raw.lines()
            .find(|l| l.to_ascii_lowercase().starts_with("via:"))
            .and_then(|via| via.split("branch=").nth(1))
            .map(|b| b.split([';', ' ']).next().unwrap_or("").to_string())
    };
    let bye = trace
        .iter()
        .find(|m| m.direction == "outbound" && m.method_or_status == "BYE")
        .unwrap();
    let ok = trace
        .iter()
        .find(|m| {
            m.direction == "inbound" && m.method_or_status == "200" && m.cseq.ends_with("BYE")
        })
        .unwrap();
    assert_eq!(
        branch(&bye.raw_message),
        branch(&ok.raw_message),
        "\n{ladder}"
    );
    assert!(
        bye.timestamp <= ok.timestamp,
        "the BYE before its answer\n{ladder}"
    );
    stack.finish().await;
}

/// What a call leaves behind (#344): once calls have ended, whichever side
/// hung up, the node holds no dialog, call or media session for them, and
/// the Call-IDs it remembers for tracing stay within a margin rather than
/// growing by two a call. Before the fix the B-legs' dialogs stayed in the
/// UAC's own store and every Call-ID stayed in the map, for a day's soak
/// about 1.2 KB a call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ended_calls_leave_nothing_behind() {
    const CALLS: usize = 40;
    // One source placing calls back to back, challenged each time.
    let Some(stack) = Stack::start_with("[security]\nmax_invite_cps_per_source = 0\n", 40).await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    for n in 0..CALLS {
        let alice_call = alice_phone.call(&bob.extension).await.unwrap();
        let Some(incoming) = bob_phone.incoming(SOON).await else {
            stack
                .fail(&format!("call {n}: Bob's phone did not ring"), &alice_phone)
                .await;
        };
        let bob_call = incoming.answer().await.unwrap();
        if let Err(e) = alice_call.answered(SOON).await {
            stack
                .fail(
                    &format!("call {n}: Alice's call was not answered: {e}"),
                    &alice_phone,
                )
                .await;
        }
        if n % 2 == 0 {
            alice_call.hangup().await.unwrap();
            assert!(bob_call.ended(SOON).await, "call {n}: Bob's leg ends");
        } else {
            bob_call.hangup().await.unwrap();
            assert!(alice_call.ended(SOON).await, "call {n}: Alice's leg ends");
        }
    }

    let deadline = tokio::time::Instant::now() + SOON;
    let held = loop {
        let held = stack.get("/status").await["held"].clone();
        let ended = held["dialogs"] == 0 && held["calls"] == 0 && held["media_sessions"] == 0;
        if ended || tokio::time::Instant::now() > deadline {
            break held;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(held["dialogs"], 0, "{held}");
    assert_eq!(held["calls"], 0, "{held}");
    assert_eq!(held["media_sessions"], 0, "{held}");
    // Swept once past twice the calls up and a margin of 64: never the 80
    // two legs a call would leave.
    let ids = held["sip_call_ids"].as_u64().unwrap();
    assert!(ids <= 66, "{CALLS} ended calls left {ids} Call-IDs: {held}");

    let metrics = http_get(&loopback(stack.http, "/metrics")).await.unwrap().1;
    assert!(
        metrics.contains("kind=\"dialogs\"} 0"),
        "the gauge says what /status does"
    );

    stack.finish().await;
}

/// A call that rings two phones and is answered on one leaves no early
/// dialog behind (#372). The node's UAC keeps an early dialog for each
/// phone that rings; the one cancelled stayed in the dialog store for
/// good — as did the ringing dialog of every join to the conference
/// server, which rang under one To tag and answered under another.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rung_and_cancelled_legs_leave_no_dialog_behind() {
    const CALLS: usize = 6;
    let Some(stack) = Stack::start_with("[security]\nmax_invite_cps_per_source = 0\n", 40).await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let desk = stack.phone(&bob).await;
    let mobile = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(desk.register().await.unwrap(), 200);
    assert_eq!(mobile.register().await.unwrap(), 200);

    for n in 0..CALLS {
        let alice_call = alice_phone.call(&bob.extension).await.unwrap();
        let (on_desk, on_mobile) = tokio::join!(desk.incoming(SOON), mobile.incoming(SOON));
        let (Some(on_desk), Some(on_mobile)) = (on_desk, on_mobile) else {
            stack
                .fail(
                    &format!("call {n}: Bob's phones did not both ring"),
                    &alice_phone,
                )
                .await;
        };
        // The desk answers; the mobile is cancelled.
        let ringing = on_mobile.leave_ringing();
        let bob_call = on_desk.answer().await.unwrap();
        alice_call.answered(SOON).await.unwrap();
        let _ = ringing.ended(SOON).await;
        if n % 2 == 0 {
            alice_call.hangup().await.unwrap();
            assert!(bob_call.ended(SOON).await, "call {n}: Bob's leg ends");
        } else {
            bob_call.hangup().await.unwrap();
            assert!(alice_call.ended(SOON).await, "call {n}: Alice's leg ends");
        }
    }

    let deadline = tokio::time::Instant::now() + SOON;
    let held = loop {
        let held = stack.get("/status").await["held"].clone();
        if (held["dialogs"] == 0 && held["calls"] == 0) || tokio::time::Instant::now() > deadline {
            break held;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(held["dialogs"], 0, "{CALLS} rung calls left: {held}");
    assert_eq!(held["calls"], 0, "{held}");
    stack.finish().await;
}

/// Phones behind NAT (`fcp_sip::nat`): each names, in its Contact and its
/// SDP, an address it cannot be reached at — loopback, as Anvil did on D8 —
/// and is seen at another. The callee is rung where it registered from,
/// each side's RTP is learned from where it comes from, and requests
/// inside the call reach both.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phones_behind_nat_ring_hear_each_other_and_hang_up() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let nowhere: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let alice_phone = Phone::behind_nat(
        "127.0.0.2:0",
        nowhere,
        &alice.username,
        &alice.password,
        REALM,
        stack.sip,
    )
    .await
    .unwrap();
    let bob_phone = Phone::behind_nat(
        "127.0.0.3:0",
        nowhere,
        &bob.username,
        &bob.password,
        REALM,
        stack.sip,
    )
    .await
    .unwrap();
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let why = format!(
            "Bob's phone, registered from 127.0.0.3 naming 127.0.0.1, did not ring; \
             Alice's call is {:?}",
            alice_call.phase()
        );
        stack.fail(&why, &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Both talk at once, as phones send from the answer: a leg's address
    // is learned from what it sends, so a silent one is reached only once
    // it speaks.
    alice_call.media().play_tone(440.0, 1500);
    bob_call.media().play_tone(880.0, 1500);
    if !bob_call.media().heard(440, SOON).await {
        let why = format!(
            "Bob does not hear Alice's 440 Hz (received {} packets)",
            bob_call.media().packets_received()
        );
        stack.fail(&why, &bob_phone).await;
    }
    if !alice_call.media().heard(880, SOON).await {
        let why = format!(
            "Alice does not hear Bob's 880 Hz (received {} packets)",
            alice_call.media().packets_received()
        );
        stack.fail(&why, &alice_phone).await;
    }

    // Inside the call: an offer relayed each way reaches the far phone,
    // Bob's BYE reaches Alice where her INVITE was seen from, and on a
    // second call Alice's BYE reaches Bob where his answer was.
    let (code, _) = alice_call.reinvite_answered("sendrecv").await.unwrap();
    assert_eq!(code, 200, "Alice's re-INVITE is answered");
    if bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .is_none()
    {
        stack
            .fail("Bob's phone did not get Alice's re-INVITE", &bob_phone)
            .await;
    }
    let (code, _) = bob_call.reinvite_answered("sendrecv").await.unwrap();
    assert_eq!(code, 200, "Bob's re-INVITE is answered");
    if alice_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .is_none()
    {
        stack
            .fail("Alice's phone did not get Bob's re-INVITE", &alice_phone)
            .await;
    }
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    if alice_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
        .await
        .is_none()
    {
        stack
            .fail("Alice's phone did not get the BYE", &alice_phone)
            .await;
    }
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let bob_call = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings again")
        .answer()
        .await
        .unwrap();
    alice_call.answered(SOON).await.unwrap();
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    if bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
        .await
        .is_none()
    {
        stack
            .fail("Bob's phone did not get the BYE", &bob_phone)
            .await;
    }
    assert!(bob_call.ended(SOON).await);

    stack.finish().await;
}

/// Two phones behind one router, as on D8 (`docs/NAT_AND_TLS.md`): one
/// public address, each phone naming an address it cannot be reached at
/// (no SIP ALG), on different codecs so the node transcodes. The legs'
/// addresses are learned from their first packets; with one port for both
/// legs the first packet from the shared address was taken as leg A's
/// whoever sent it, and transcoding ran backwards. Both hear each other,
/// and a hold's music reaches the held phone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_phones_behind_one_address_hear_each_other() {
    behind_one_address(false).await;
}

/// The same router with a SIP ALG: each phone's Contact and SDP name the
/// shared address and the phone's own port.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_phones_behind_one_address_with_an_alg_hear_each_other() {
    behind_one_address(true).await;
}

async fn behind_one_address(alg: bool) {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let shared = "127.0.0.2";
    let named: std::net::IpAddr = if alg { shared } else { "127.0.0.1" }.parse().unwrap();
    let bind = format!("{shared}:0");
    let alice_phone = Phone::behind_nat(
        &bind,
        named,
        &alice.username,
        &alice.password,
        REALM,
        stack.sip,
    )
    .await
    .unwrap();
    let bob_phone = Phone::behind_nat(&bind, named, &bob.username, &bob.password, REALM, stack.sip)
        .await
        .unwrap();
    alice_phone.prefer_codec(fcp_testkit::media::Codec::Pcma);
    bob_phone.prefer_codec(fcp_testkit::media::Codec::Pcmu);
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack
            .fail(
                "Bob's phone, behind Alice's address, did not ring",
                &alice_phone,
            )
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Bob speaks first, as the VVX did.
    bob_call.media().play_tone(880.0, 3000);
    tokio::time::sleep(Duration::from_millis(200)).await;
    alice_call.media().play_tone(440.0, 3000);
    if !bob_call.media().heard(440, SOON).await {
        let why = format!(
            "Bob does not hear Alice's 440 Hz across PCMA to PCMU (received {} packets)",
            bob_call.media().packets_received()
        );
        stack.fail(&why, &bob_phone).await;
    }
    if !alice_call.media().heard(880, SOON).await {
        let why = format!(
            "Alice does not hear Bob's 880 Hz across PCMU to PCMA (received {} packets)",
            alice_call.media().packets_received()
        );
        stack.fail(&why, &alice_phone).await;
    }

    // Alice holds: the music reaches Bob, the held phone, and not Alice.
    alice_call.media().stop_tone();
    bob_call.media().stop_tone();
    assert_eq!(alice_call.hold().await.unwrap(), 200);
    if !bob_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await {
        stack
            .fail("Bob, held, does not hear the hold music", &bob_phone)
            .await;
    }
    assert!(
        !alice_call
            .media()
            .heard(DEFAULT_MUSIC_HZ, Duration::from_secs(1))
            .await,
        "the holder hears the held phone's music"
    );
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

/// A phone resuming a call offers its codecs again, perhaps in another
/// order: in D8 a VVX answered PCMU and resumed offering G.722 first. The
/// node answers a resume itself under hold music, and answered in the
/// offer's first codec while its media session went on decoding the old
/// one, so Anvil heard white noise. It answers in the codec the leg uses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_resume_offering_another_codec_first_keeps_the_call_s_codec() {
    use fcp_testkit::media::Codec;
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    alice_phone.prefer_codec(Codec::Pcma);
    bob_phone.prefer_codec(Codec::Pcmu);
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Bob holds; Alice hears the music.
    assert_eq!(bob_call.hold().await.unwrap(), 200);
    if !alice_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await {
        stack
            .fail("Alice, held, does not hear the hold music", &alice_phone)
            .await;
    }

    // Bob resumes offering PCMA first: answered in PCMU, the call's.
    let (code, answer) = bob_call
        .reinvite_offering_first("sendrecv", Codec::Pcma)
        .await
        .unwrap();
    assert_eq!(code, 200);
    let media = answer
        .lines()
        .find(|l| l.starts_with("m=audio"))
        .unwrap_or_default();
    assert_eq!(
        media.split_whitespace().nth(3),
        Some("0"),
        "the resume was answered in another codec than the call's:\n{answer}"
    );
    assert!(!answer.contains("PCMA"), "{answer}");

    bob_call.media().play_tone(880.0, 3000);
    if !alice_call.media().heard(880, SOON).await {
        stack
            .fail("Alice does not hear Bob after he resumed", &alice_phone)
            .await;
    }
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

/// Two registered phones with the callee answered: the start of most
/// feature scenarios.
async fn answered_call(stack: &Stack) -> (Phone, fcp_testkit::Call, Phone, fcp_testkit::Call) {
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        let why = format!(
            "Bob's phone did not ring; Alice's call is {:?}",
            alice_call.phase()
        );
        stack.fail(&why, &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    if let Err(e) = alice_call.answered(SOON).await {
        let why = format!("Alice's call was not answered: {e}");
        stack.fail(&why, &alice_phone).await;
    }
    (alice_phone, alice_call, bob_phone, bob_call)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_and_decline_reach_the_caller() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    for (code, reason) in [(486, "Busy Here"), (603, "Decline")] {
        let call = alice_phone.call(&bob.extension).await.unwrap();
        let incoming = bob_phone.incoming(SOON).await.expect("Bob's phone rings");
        incoming.reject(code, reason).await.unwrap();
        match call.answered(SOON).await {
            Err(err) if err.to_string().contains(&code.to_string()) => {}
            other => {
                let why = format!("{code} {reason} did not reach the caller: {other:?}");
                stack.fail(&why, &alice_phone).await;
            }
        }
    }

    // Each rejection is on the record as what the callee sent, not as a
    // guess from the failure text.
    let cdrs = stack.cdrs(2, SOON).await;
    let by_status = |code: u64| {
        cdrs.iter()
            .find(|c| c["sip_status_code"].as_u64() == Some(code))
            .unwrap_or_else(|| panic!("no record with status {code}: {cdrs:#?}"))
    };
    let busy = by_status(486);
    assert_eq!(busy["disposition"], "Busy", "{busy}");
    assert_eq!(busy["hangup_cause"], "UserBusy", "{busy}");
    assert_eq!(busy["hangup_party"], "Callee", "{busy}");
    let declined = by_status(603);
    assert_eq!(declined["disposition"], "Failed", "{declined}");
    assert_eq!(declined["hangup_cause"], "CallRejected", "{declined}");
    assert_eq!(declined["hangup_party"], "Callee", "{declined}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unanswered_call_times_out_and_the_callee_is_cancelled() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let call = alice_phone.call(&bob.extension).await.unwrap();
    let ringing = bob_phone
        .incoming(SOON)
        .await
        .expect("Bob's phone rings")
        .leave_ringing();
    // [b2bua.forking] timeout = 3: the caller hears the timeout (the
    // B2BUA answers 480 for a callee who did not pick up), the callee's
    // ringing INVITE is cancelled.
    match call.answered(Duration::from_secs(10)).await {
        Err(err) if err.to_string().contains("480") || err.to_string().contains("408") => {}
        other => {
            let why = format!("no ring timeout reached the caller: {other:?}");
            stack.fail(&why, &alice_phone).await;
        }
    }
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Cancelled { .. }))
            .await
            .is_some(),
        "Bob's phone was cancelled"
    );
    assert!(ringing.ended(SOON).await);

    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["disposition"], "NoAnswer", "{cdr}");
    assert_eq!(cdr["hangup_cause"], "NoAnswer", "{cdr}");
    assert!(cdr["answer_time_epoch"].is_null(), "{cdr}");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_record_survives_the_database_going_away_under_the_call() {
    let stack = stack_or_skip!();
    let (_alice_phone, alice_call, bob_phone, _bob_call) = answered_call(&stack).await;

    // The CDR table goes away while the call is up: the database refuses
    // the record when the call ends (docs/CDR_PIPELINE.md §6).
    stack
        .seed
        .execute("ALTER TABLE cdrs RENAME TO cdrs_away")
        .await;
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(
        bob_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Bye { .. }))
            .await
            .is_some(),
        "Bob's phone gets BYE"
    );
    // Long enough for the shipper to have tried and backed off.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // The table comes back; the spooled record lands, once.
    stack
        .seed
        .execute("ALTER TABLE cdrs_away RENAME TO cdrs")
        .await;
    let cdrs = stack.cdrs(1, Duration::from_secs(20)).await;
    assert_eq!(cdrs.len(), 1, "{cdrs:#?}");
    assert_eq!(cdrs[0]["disposition"], "Answered", "{}", cdrs[0]);
    assert_eq!(cdrs[0]["hangup_party"], "Caller", "{}", cdrs[0]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(stack.cdrs(1, SOON).await.len(), 1, "no second copy");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_to_an_unregistered_user_fails() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    let call = alice_phone.call(&carol.extension).await.unwrap();
    let err = call.answered(SOON).await.expect_err("nobody registered");
    assert!(
        err.to_string().contains("480") || err.to_string().contains("404"),
        "{err}"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hold_and_resume_are_relayed_to_the_far_end() {
    // Without hold music the node passes a hold on to the far phone
    // (with it, the node answers it: docs/ENTERPRISE_CALLING.md §3.3).
    let Some(stack) = Stack::start_configured("hold_music = false", "", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let (_alice_phone, alice_call, bob_phone, _bob_call) = answered_call(&stack).await;

    assert_eq!(alice_call.hold().await.unwrap(), 200);
    let held = bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .expect("Bob's phone gets the hold re-INVITE");
    let PhoneEvent::ReInvite { sdp, .. } = held else {
        unreachable!()
    };
    assert!(sdp.contains("a=sendonly"), "hold SDP relayed: {sdp}");

    assert_eq!(alice_call.resume().await.unwrap(), 200);
    let resumed = bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::ReInvite { .. }))
        .await
        .expect("Bob's phone gets the resume re-INVITE");
    let PhoneEvent::ReInvite { sdp, .. } = resumed else {
        unreachable!()
    };
    assert!(sdp.contains("a=sendrecv"), "resume SDP relayed: {sdp}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dtmf_crosses_as_rfc_2833_and_as_info() {
    let stack = stack_or_skip!();
    let (_alice_phone, alice_call, bob_phone, bob_call) = answered_call(&stack).await;

    // RFC 2833 in the media.
    alice_call.media().send_dtmf('5');
    if bob_call.media().heard_dtmf(SOON).await != Some('5') {
        let why = format!(
            "Bob did not hear the digit (Alice sent {} packets, Bob received {})",
            alice_call.media().packets_sent(),
            bob_call.media().packets_received()
        );
        stack.fail(&why, &_alice_phone).await;
    }

    // SIP INFO dtmf-relay, relayed to the far phone.
    assert_eq!(alice_call.send_info_dtmf('7').await.unwrap(), 200);
    let info = bob_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::Info { .. }))
        .await
        .expect("Bob's phone gets the INFO");
    let PhoneEvent::Info { body, .. } = info else {
        unreachable!()
    };
    assert!(body.contains("Signal=7"), "{body}");

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relayed_blind_transfer_moves_the_caller_to_a_third_phone() {
    // The REFER relayed to the far phone, as before the node carried
    // transfers out itself (docs/ENTERPRISE_CALLING.md §3.2).
    let Some(stack) = Stack::start_with("[b2bua.transfer]\nexecute = false\n", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let (alice_phone, alice_call, bob_phone, bob_call) = answered_call(&stack).await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let carol_phone = stack.phone(&carol).await;
    assert_eq!(carol_phone.register().await.unwrap(), 200);
    alice_phone.follow_refers(true);

    // Bob transfers Alice to Carol: the REFER reaches Alice's phone,
    // which dials Carol.
    let refer_to = format!("sip:{}@{}", carol.extension, stack.sip);
    assert_eq!(bob_call.refer(&refer_to).await.unwrap(), 202);
    assert!(
        alice_phone
            .expect(SOON, |e| matches!(e, PhoneEvent::Refer { .. }))
            .await
            .is_some(),
        "Alice's phone gets the REFER"
    );
    let carol_call = carol_phone
        .incoming(Duration::from_secs(10))
        .await
        .expect("Carol's phone rings")
        .answer()
        .await
        .unwrap();

    // Alice and Carol hear each other; Bob's leg is dropped.
    carol_call.media().play_tone(660.0, 1500);
    let alice_new = alice_phone
        .expect(SOON, |e| matches!(e, PhoneEvent::Answered { .. }))
        .await
        .expect("Alice's new call is answered");
    let _ = alice_new;
    assert!(
        bob_phone
            .expect(Duration::from_secs(10), |e| matches!(
                e,
                PhoneEvent::Bye { .. }
            ))
            .await
            .is_some(),
        "Bob's phone gets BYE once the transfer succeeds"
    );
    assert!(alice_call.ended(SOON).await, "Alice's old leg is over");

    assert_eq!(carol_call.hangup().await.unwrap(), 200);

    // Two call records, and the new call knows the call it came from
    // (docs/CDR_PIPELINE.md §5): the Refer-To this call manager relayed
    // carried it, and Alice's phone copied it into the new INVITE.
    let records = stack.cdrs(2, SOON).await;
    let with_bob = records
        .iter()
        .find(|r| r["to_uri"].as_str().unwrap_or("").contains(":1002@"))
        .unwrap_or_else(|| panic!("no record of the call to Bob: {records:#?}"));
    let with_carol = records
        .iter()
        .find(|r| r["to_uri"].as_str().unwrap_or("").contains(":1003@"))
        .unwrap_or_else(|| panic!("no record of the call to Carol: {records:#?}"));
    assert_eq!(
        with_carol["correlation_id"], with_bob["call_id"],
        "the transferred call points at the call it left: {with_carol}"
    );
    assert!(with_bob["correlation_id"].is_null(), "{with_bob}");
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_queued_caller_leaves_a_call_record_and_a_queue_record_under_one_id() {
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true

[queue.queues.support]
name = "support"
extension = "7000"
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    // Alice calls the queue; it answers her and nobody takes the call.
    let call = alice_phone.call("7000").await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(&format!("the queue did not answer: {e}"), &alice_phone)
            .await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);

    // The call and its time in the queue, recorded under one call id; the
    // default listing shows the call alone.
    let records = stack.cdrs_at("/cdrs?kind=all", 2, SOON).await;
    assert_eq!(records.len(), 2, "{records:#?}");
    let call_record = records
        .iter()
        .find(|r| r["kind"] == "call")
        .expect("call record");
    let queue_record = records
        .iter()
        .find(|r| r["kind"] == "queue")
        .expect("queue record");
    assert_eq!(
        call_record["call_id"], queue_record["call_id"],
        "{records:#?}"
    );
    assert_eq!(call_record["disposition"], "Answered", "{call_record}");
    assert_eq!(queue_record["disposition"], "Cancelled", "{queue_record}");
    let calls_only = stack.cdrs(1, SOON).await;
    assert_eq!(calls_only.len(), 1, "{calls_only:#?}");
    assert_eq!(calls_only[0]["kind"], "call");

    // And the call is gone from the live calls.
    let live = stack.get("/calls").await;
    assert_eq!(live["calls"].as_array().map(Vec::len), Some(0), "{live}");
    stack.finish().await;
}

/// A renamed user's phone keeps its SIP name (docs/USERS.md §6): it
/// registers with the credentials it had, calls out as before, and is
/// reached by its extension.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_renamed_users_phone_registers_calls_and_is_reached_as_before() {
    use fcp_core::CredentialStore;
    let Some(stack) = Stack::start().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let storage = stack.seed.storage();
    let user = storage
        .get_user_in_tenant("alice", &stack.seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let renamed = storage.rename_user(&user, "alice.smith").await.unwrap();
    assert_eq!(renamed.username, "alice.smith");
    assert_eq!(renamed.sip_name(), "alice");

    let alice_phone = stack.phone(&alice).await;
    assert_eq!(
        alice_phone.register().await.unwrap(),
        200,
        "the renamed user's phone registers with what it had"
    );
    alice_phone.auto_answer(true);
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    bob_phone.auto_answer(true);

    let out = alice_phone.call("1002").await.unwrap();
    if let Err(e) = out.answered(SOON).await {
        stack
            .fail(
                &format!("the renamed user's call was not answered: {e}"),
                &alice_phone,
            )
            .await;
    }
    out.hangup().await.unwrap();
    for dialed in ["1001"] {
        let call = bob_phone.call(dialed).await.unwrap();
        if let Err(e) = call.answered(SOON).await {
            stack
                .fail(
                    &format!("{dialed} did not reach the renamed user: {e}"),
                    &bob_phone,
                )
                .await;
        }
        call.hangup().await.unwrap();
    }
    stack.finish().await;
}

/// The queue system's logged-in agents, by id.
async fn logged_in_agents(stack: &Stack) -> Vec<String> {
    stack.get("/agents").await["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a["agent_id"].as_str().map(String::from))
        .collect()
}

/// A queue may take its agents from a group (docs/USERS.md §5): a member
/// logs in with the feature code; someone outside the group is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_groups_member_logs_in_to_the_queue_that_takes_its_agents_from_it() {
    use fcp_core::group::{Group, GroupMember, GroupStore};
    use fcp_core::{CredentialStore, QueueConfigStore};
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let dana = stack.seed.user("dana", "dana-pw", "1004").await;
    let eve = stack.seed.user("eve", "eve-pw", "1005").await;
    let storage = stack.seed.storage();
    let tenant = stack.seed.tenant_id;
    let group = Group::new(tenant, "NOC");
    storage.put_group(&group).await.unwrap();
    let dana_id = storage
        .get_user_in_tenant("dana", &tenant)
        .await
        .unwrap()
        .unwrap()
        .id;
    storage
        .add_member(&GroupMember {
            group_id: group.id,
            user_id: dana_id,
            tenant_id: tenant,
            added_at: std::time::SystemTime::now(),
        })
        .await
        .unwrap();
    let mut queue = fcp_core::QueueConfigRecord::new("noc".into(), "7100".into(), tenant);
    queue.agent_group_id = Some(group.id);
    storage.put_queue_config(queue).await.unwrap();
    let (code, reloaded) = stack.post("/queues/reload", "").await;
    assert_eq!(code, 200, "{reloaded}");

    // Eve is no member: refused.
    let eve_phone = stack.phone(&eve).await;
    assert_eq!(eve_phone.register().await.unwrap(), 200);
    let refused = eve_phone.call("*50").await.unwrap();
    assert_eq!(refused.failed(SOON).await, Some(404));
    assert!(logged_in_agents(&stack).await.is_empty());

    // Dana is: logged in to the queue.
    let dana_phone = stack.phone(&dana).await;
    assert_eq!(dana_phone.register().await.unwrap(), 200);
    let _login = dana_phone.call("*50").await.unwrap();
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let now = logged_in_agents(&stack).await;
        if now.iter().any(|a| a == "dana") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dana never logged in: {now:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    stack.finish().await;
}

/// A caller who hangs up while the queue rings its agent stops the ring:
/// the agent's phone is cancelled at once. The queue used to abort its ring
/// task as it signalled it, so no CANCEL was sent — the phone rang on until
/// the B2BUA's own ring timeout (3 s here), and one picked up in the
/// meantime was answered onto nobody.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_leaving_the_queue_stops_the_agents_ring() {
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true

[queue.queues.support]
name = "support"
extension = "7000"
agents = ["1002"]
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let (_alice, alice_phone, _bob, bob_phone) = two_phones(&stack).await;
    let (code, login) = stack.post("/agents/1002/login", "{}").await;
    assert_eq!(code, 200, "{login}");

    let call = alice_phone.call("7000").await.unwrap();
    call.answered(SOON).await.unwrap();
    let Some(ringing) = bob_phone.incoming(Duration::from_secs(10)).await else {
        stack.fail("the agent was not rung", &alice_phone).await;
    };
    let ringing = ringing.leave_ringing();
    assert_eq!(call.hangup().await.unwrap(), 200);
    if bob_phone
        .expect(Duration::from_millis(1500), |e| {
            matches!(e, PhoneEvent::Cancelled { .. })
        })
        .await
        .is_none()
    {
        stack
            .fail(
                "the agent's phone rang on after the caller left",
                &bob_phone,
            )
            .await;
    }
    assert!(ringing.ended(SOON).await);
    stack.finish().await;
}

/// Two tenants each have a queue called `sales` at 7000 with their own
/// agent at 1001 (docs/TENANT_SCOPE.md §4b). Each caller's call waits in
/// its own tenant's queue and rings its own tenant's agent. Before, the
/// second `sales` stopped the node loading any queue from the database, and
/// a queue rang its agent's extension in the node's default tenant.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_tenants_queues_of_one_name_ring_their_own_agents() {
    use fcp_core::storage::QueueStore;
    use fcp_core::QueueConfigStore;
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let acme = stack.seed.tenant_id;
    let globex = stack.seed.tenant("globex", "globex.e2e.local").await;
    let ann = stack.seed.user("ann", "ann-pw", "1001").await;
    let carl = stack.seed.user("carl", "carl-pw", "1003").await;
    let ben = stack
        .seed
        .user_in(globex, "ben-globex", "ben-pw", "1001")
        .await;
    let cara = stack
        .seed
        .user_in(globex, "cara-globex", "cara-pw", "1003")
        .await;
    let storage = stack.seed.storage();
    for tenant in [acme, globex] {
        let mut sales = fcp_core::QueueConfigRecord::new("sales".into(), "7000".into(), tenant);
        sales.agents = vec!["1001".into()];
        storage.put_queue_config(sales).await.unwrap();
    }
    let (code, reloaded) = stack.post("/queues/reload", "").await;
    assert_eq!(code, 200, "{reloaded}");
    assert_eq!(reloaded["status"], "reloaded", "{reloaded}");

    let globex_phone = |user: &SeededUser| {
        let user = user.clone();
        let sip = stack.sip;
        async move {
            Phone::new(&user.username, &user.password, "globex.e2e.local", sip)
                .await
                .unwrap()
        }
    };
    let ann_phone = stack.phone(&ann).await;
    let carl_phone = stack.phone(&carl).await;
    let ben_phone = globex_phone(&ben).await;
    let cara_phone = globex_phone(&cara).await;
    for phone in [&ann_phone, &carl_phone, &ben_phone, &cara_phone] {
        assert_eq!(phone.register().await.unwrap(), 200);
    }

    // Each agent logs in to its own tenant's `sales`.
    let _ann_login = ann_phone.call("*50").await.unwrap();
    let _ben_login = ben_phone.call("*50").await.unwrap();
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        let acme_agents = logged_in_agents(&stack).await;
        let globex_agents = storage.load_agents(&globex).await.unwrap();
        if acme_agents.iter().any(|a| a == "ann")
            && globex_agents
                .iter()
                .any(|a| a.id == "ben-globex" && a.queues == ["sales"])
        {
            assert!(!acme_agents.iter().any(|a| a == "ben-globex"));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the agents never logged in: {acme_agents:?} {globex_agents:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Cara (globex) calls 7000: Ben is rung, Ann is not.
    let cara_call = cara_phone.call("7000").await.unwrap();
    if let Err(e) = cara_call.answered(SOON).await {
        stack
            .fail(&format!("globex's queue did not answer: {e}"), &cara_phone)
            .await;
    }
    let Some(incoming) = ben_phone.incoming(Duration::from_secs(10)).await else {
        stack
            .fail("globex's agent was not rung by globex's queue", &cara_phone)
            .await;
    };
    assert!(
        ann_phone
            .incoming(Duration::from_millis(500))
            .await
            .is_none(),
        "acme's agent was rung for globex's caller"
    );
    let ben_call = incoming.answer().await.unwrap();
    cara_call.media().play_tone(440.0, 1000);
    assert!(ben_call.media().heard(440, SOON).await);
    assert_eq!(cara_call.hangup().await.unwrap(), 200);
    assert!(ben_call.ended(SOON).await);

    // Carl (acme) calls 7000: Ann is rung, Ben is not.
    let carl_call = carl_phone.call("7000").await.unwrap();
    carl_call.answered(SOON).await.unwrap();
    let Some(incoming) = ann_phone.incoming(Duration::from_secs(10)).await else {
        stack
            .fail("acme's agent was not rung by acme's queue", &carl_phone)
            .await;
    };
    assert!(
        ben_phone
            .incoming(Duration::from_millis(500))
            .await
            .is_none(),
        "globex's agent was rung for acme's caller"
    );
    let ann_call = incoming.answer().await.unwrap();
    assert_eq!(carl_call.hangup().await.unwrap(), 200);
    if !ann_call.ended(SOON).await {
        stack
            .fail(
                "acme's agent was left on after the caller hung up",
                &ann_phone,
            )
            .await;
    }
    stack.finish().await;
}

// ── Recording consent (docs/COMPLIANCE_FOLLOWUPS.md §8, §10) ───────────

/// The prompt's tone: neither phone plays it, so hearing it is hearing the
/// announcement.
const PROMPT_HZ: u32 = 1000;
/// The continuing notice (`fcp_core::compliance::NOTICE_TONE_HZ`).
const NOTICE_HZ: u32 = 1400;

/// The tenant announces recordings with a prompt at `audio` (a file that
/// need not exist), doing `on_failure` when it cannot, with a notice every
/// `notice` seconds.
async fn announce_recordings(
    stack: &Stack,
    audio: &std::path::Path,
    on_failure: fcp_core::compliance::ConsentFailureAction,
    notice: Option<u32>,
) {
    use fcp_core::compliance::{CompliancePolicy, ComplianceStore, RecordingConsentMode};
    use fcp_core::prompt::{Prompt, PromptCategory, PromptStore};

    let pool = stack.seed.storage().pool().clone();
    let prompt = Prompt::new_file(
        stack.seed.tenant_id,
        "recording-consent",
        PromptCategory::Custom,
        "en-US",
        audio.to_path_buf(),
    );
    fcp_storage_postgres::prompt::PostgresPromptStore::new(pool.clone())
        .put_prompt(prompt.clone())
        .await
        .unwrap();
    let mut policy = CompliancePolicy::defaults(stack.seed.tenant_id);
    policy.recording_consent = RecordingConsentMode::Announce;
    policy.recording_consent_prompt_id = Some(prompt.id.to_string());
    policy.recording_consent_on_failure = on_failure;
    policy.recording_notice_interval_secs = notice;
    fcp_storage_postgres::compliance::PostgresComplianceStore::new(pool)
        .put_compliance_policy(&policy)
        .await
        .unwrap();
}

/// A prompt that is a 1.5 s tone at [`PROMPT_HZ`], under the stack's
/// directory.
fn tone_prompt(stack: &Stack) -> std::path::PathBuf {
    let path = stack.dir().join("consent.wav");
    fcp_testkit::wav::write_tone(&path, PROMPT_HZ as f32, 1500).unwrap();
    path
}

/// Every WAV file under `dir`.
fn wav_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "wav") {
                found.push(path);
            }
        }
    }
    found
}

/// The file of the first recording of a live call, once its row is written
/// — which is once its recorder runs.
async fn recording_started(stack: &Stack, within: Duration) -> Option<std::path::PathBuf> {
    use fcp_core::recording::CallRecordingStore;
    let store = fcp_storage_postgres::recording::PostgresRecordingStore::new(
        stack.seed.storage().pool().clone(),
    );
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let calls = stack.get("/calls").await;
        for call in calls["calls"].as_array().cloned().unwrap_or_default() {
            let Some(id) = call["call_id"]
                .as_str()
                .and_then(|id| id.parse::<fcp_core::CallId>().ok())
            else {
                continue;
            };
            if let Some(row) = store
                .call_recordings_for(&id.to_string())
                .await
                .unwrap()
                .pop()
            {
                return Some(row.path.into());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The call recordings rows of the record `cdr`.
async fn recording_rows(
    stack: &Stack,
    cdr: &serde_json::Value,
) -> Vec<fcp_core::recording::CallRecording> {
    use fcp_core::recording::CallRecordingStore;
    fcp_storage_postgres::recording::PostgresRecordingStore::new(
        stack.seed.storage().pool().clone(),
    )
    .call_recordings_for(cdr["call_id"].as_str().unwrap())
    .await
    .unwrap()
}

/// Alice (1001) and Bob (1002), their phones registered.
async fn two_phones(stack: &Stack) -> (SeededUser, Phone, SeededUser, Phone) {
    two_phones_after(stack, "SELECT 1").await
}

/// As [`two_phones`], running `sql` once the users exist and before their
/// phones register.
async fn two_phones_after(stack: &Stack, sql: &str) -> (SeededUser, Phone, SeededUser, Phone) {
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    stack.seed.execute(sql).await;
    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    (alice, alice_phone, bob, bob_phone)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_who_records_every_call_hears_the_announcement_before_the_recording() {
    use fcp_core::compliance::ConsentFailureAction;

    let stack = stack_or_skip!();
    let prompt = tone_prompt(&stack);
    announce_recordings(&stack, &prompt, ConsentFailureAction::Record, Some(5)).await;
    // Before the phones register: the node caches the users it has seen.
    let (_alice, alice_phone, bob, bob_phone) = two_phones_after(
        &stack,
        "UPDATE users SET media_preferences = jsonb_set(media_preferences, \
         '{record_all_calls}', 'true') WHERE username = 'alice'",
    )
    .await;

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // Both parties hear the announcement once the call is answered.
    let (alice_heard, bob_heard) = tokio::join!(
        alice_call.media().heard(PROMPT_HZ, SOON),
        bob_call.media().heard(PROMPT_HZ, SOON)
    );
    if !(alice_heard && bob_heard) {
        let why = format!("announcement heard by Alice {alice_heard}, by Bob {bob_heard}");
        stack.fail(&why, &alice_phone).await;
    }

    // The recording starts after it, and hears both of them.
    let Some(file) = recording_started(&stack, SOON).await else {
        stack.fail("no recording was started", &alice_phone).await;
    };
    // One after the other, so each is plain in the recording.
    alice_call.media().play_tone(440.0, 1000);
    assert!(bob_call.media().heard(440, SOON).await);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    bob_call.media().play_tone(880.0, 1000);
    assert!(alice_call.media().heard(880, SOON).await);
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // While it runs, both hear the notice.
    let (alice_notice, bob_notice) = tokio::join!(
        alice_call.media().heard(NOTICE_HZ, Duration::from_secs(8)),
        bob_call.media().heard(NOTICE_HZ, Duration::from_secs(8))
    );
    assert!(
        alice_notice && bob_notice,
        "notice heard by Alice {alice_notice}, by Bob {bob_notice}"
    );

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);

    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["recording_consent"], "announced", "{cdr}");
    assert_eq!(cdr["recording_url"], file.display().to_string(), "{cdr}");
    let rows = recording_rows(&stack, &cdr).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].source,
        fcp_core::recording::CallRecordingSource::User
    );
    assert_eq!(rows[0].consent, fcp_core::RecordingConsent::Announced);

    let (rate, samples) = fcp_testkit::wav::read_pcm(&file).unwrap();
    assert_eq!(rate, 8000);
    assert!(
        fcp_testkit::wav::contains_tone(&samples, 440),
        "Alice is in the recording"
    );
    assert!(
        fcp_testkit::wav::contains_tone(&samples, 880),
        "Bob is in the recording"
    );
    assert!(
        !fcp_testkit::wav::contains_tone(&samples, PROMPT_HZ),
        "the announcement is not in the recording"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_that_records_every_call_records_nothing_when_the_announcement_fails_under_skip() {
    use fcp_core::compliance::ConsentFailureAction;

    let Some(stack) = Stack::start_configured("enable_recording = true", "", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    // The prompt names a file that is not there, and the node has no TTS.
    let missing = stack.dir().join("no-such-prompt.wav");
    announce_recordings(&stack, &missing, ConsentFailureAction::Skip, None).await;
    let (_alice, alice_phone, bob, bob_phone) = two_phones(&stack).await;

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // The call itself goes on.
    alice_call.media().play_tone(440.0, 1500);
    assert!(bob_call.media().heard(440, SOON).await);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);

    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["recording_consent"], "refused", "{cdr}");
    assert!(cdr["recording_url"].is_null(), "{cdr}");
    assert!(recording_rows(&stack, &cdr).await.is_empty());
    assert!(wav_files(&stack.dir().join("recordings")).is_empty());
    let metrics = http_get(&loopback(stack.http, "/metrics")).await.unwrap().1;
    assert!(
        metrics.contains("source=\"platform\",consent=\"refused\"} 1"),
        "{metrics}"
    );
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_api_records_an_anchored_call_once_its_announcement_has_played() {
    use fcp_core::compliance::ConsentFailureAction;

    let stack = stack_or_skip!();
    let prompt = tone_prompt(&stack);
    announce_recordings(&stack, &prompt, ConsentFailureAction::Record, None).await;
    let (_alice, alice_phone, bob, bob_phone) = two_phones(&stack).await;

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();
    // Nothing records this call on its own.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(wav_files(&stack.dir().join("recordings")).is_empty());

    let calls = stack.get("/calls").await;
    let call_id = calls["calls"][0]["call_id"].as_str().unwrap().to_string();
    let start = format!("/calls/{call_id}/recording/start");

    // The call as the admin API shows it; another tenant's request finds
    // neither it nor its recorder.
    let shown = stack.get(&format!("/calls/{call_id}")).await;
    assert_eq!(shown["id"], call_id.as_str(), "{shown}");
    assert_eq!(shown["state"], "Established", "{shown}");
    assert!(
        chrono::DateTime::parse_from_rfc3339(shown["established_at"].as_str().unwrap()).is_ok(),
        "{shown}"
    );
    let stranger = fcp_core::TenantId::new();
    let (code, body) = fcp_testkit::smoke::http_get(&loopback(
        stack.http,
        &format!("/calls/{call_id}?tenant_id={stranger}"),
    ))
    .await
    .unwrap();
    assert_eq!(code, 404, "{body}");
    let theirs = format!(r#"{{"tenant_id": "{stranger}"}}"#);
    assert_eq!(stack.post(&start, &theirs).await.0, 404);
    assert_eq!(
        stack
            .post(&format!("/calls/{call_id}/recording/stop"), &theirs)
            .await
            .0,
        404
    );

    // The route returns once the parties have heard the announcement.
    let (started, alice_heard, bob_heard) = tokio::join!(
        stack.post(&start, ""),
        alice_call.media().heard(PROMPT_HZ, SOON),
        bob_call.media().heard(PROMPT_HZ, SOON)
    );
    assert_eq!(started.0, 200, "{}", started.1);
    assert_eq!(started.1["consent"], "announced", "{}", started.1);
    assert!(
        alice_heard && bob_heard,
        "Alice {alice_heard}, Bob {bob_heard}"
    );
    let file = started.1["path"].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&file).exists(), "{file}");

    // Once only; a path outside the tenant's directory is refused.
    let again = stack.post(&start, "").await;
    assert_eq!(again.0, 409, "{}", again.1);
    let (code, _) = stack
        .post(&format!("/calls/call-{}/recording/start", uuid_like()), "")
        .await;
    assert_eq!(code, 404);

    let stop = format!("/calls/{call_id}/recording/stop");
    assert_eq!(stack.post(&stop, "").await.0, 200);
    assert_eq!(stack.post(&stop, "").await.0, 409, "stopped already");
    let escape = stack.post(&start, r#"{"path": "../escape.wav"}"#).await;
    assert_eq!(escape.0, 400, "{}", escape.1);

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["recording_consent"], "announced", "{cdr}");
    let rows = recording_rows(&stack, &cdr).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].source,
        fcp_core::recording::CallRecordingSource::Api
    );
    assert!(rows[0].ended_at.is_some(), "the stop closed its row");
    stack.finish().await;
}

/// A party records their own call from their app (`docs/RECORD_AND_CONTACTS.md`
/// §2): the API names them in `for_user`, the node checks they are on the
/// call, both parties hear the announcement, and only they stop it. The
/// caller's 200 OK names the call, so an app can ask for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_party_records_their_own_call_and_only_they_stop_it() {
    use fcp_core::compliance::ConsentFailureAction;

    let stack = stack_or_skip!();
    let prompt = tone_prompt(&stack);
    announce_recordings(&stack, &prompt, ConsentFailureAction::Record, None).await;
    let (alice, alice_phone, bob, bob_phone) = two_phones(&stack).await;
    let id_of = |username: String| {
        let storage = stack.seed.storage();
        let tenant = stack.seed.tenant_id;
        async move {
            fcp_core::storage::CredentialStore::get_user_in_tenant(storage, &username, &tenant)
                .await
                .unwrap()
                .unwrap()
                .id
        }
    };
    let alice_id = id_of(alice.username.clone()).await;
    let bob_id = id_of(bob.username.clone()).await;
    let carol = stack.seed.user("carol", "carol-pw", "1003").await;
    let carol_id = id_of(carol.username.clone()).await;

    let alice_call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    let calls = stack.get("/calls").await;
    let call_id = calls["calls"][0]["call_id"].as_str().unwrap().to_string();
    assert_eq!(
        alice_call.answer_header("X-FCP-Call-Id").as_deref(),
        Some(call_id.as_str()),
        "the caller's 200 OK names the call"
    );
    let start = format!("/calls/{call_id}/recording/start");
    let stop = format!("/calls/{call_id}/recording/stop");
    let as_user = |user: fcp_core::UserId| format!(r#"{{"for_user": "{user}"}}"#);

    // Someone not on the call finds no call.
    assert_eq!(stack.post(&start, &as_user(carol_id)).await.0, 404);

    let alices = as_user(alice_id);
    let (started, alice_heard, bob_heard) = tokio::join!(
        stack.post(&start, &alices),
        alice_call.media().heard(PROMPT_HZ, SOON),
        bob_call.media().heard(PROMPT_HZ, SOON)
    );
    assert_eq!(started.0, 200, "{}", started.1);
    assert_eq!(started.1["consent"], "announced", "{}", started.1);
    assert!(
        alice_heard && bob_heard,
        "Alice {alice_heard}, Bob {bob_heard}"
    );

    // Bob is on the call but did not start it; Carol is not on it.
    assert_eq!(stack.post(&stop, &as_user(carol_id)).await.0, 404);
    let bobs = stack.post(&stop, &as_user(bob_id)).await;
    assert_eq!(bobs.0, 409, "{}", bobs.1);
    assert_eq!(stack.post(&stop, &as_user(alice_id)).await.0, 200);

    // A recording the API started is not Alice's to stop.
    assert_eq!(stack.post(&start, "").await.0, 200);
    let refused = stack.post(&stop, &as_user(alice_id)).await;
    assert_eq!(refused.0, 409, "{}", refused.1);

    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    let mut sources: Vec<_> = recording_rows(&stack, &cdr)
        .await
        .into_iter()
        .map(|r| r.source)
        .collect();
    sources.sort_by_key(|s| s.as_str());
    assert_eq!(
        sources,
        [
            fcp_core::recording::CallRecordingSource::Api,
            fcp_core::recording::CallRecordingSource::OnDemand,
        ]
    );
    stack.finish().await;
}

fn uuid_like() -> &'static str {
    "00000000-0000-4000-8000-000000000000"
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_recorded_queue_call_is_announced_to_both_parties_and_mixes_both_directions() {
    use fcp_core::compliance::ConsentFailureAction;

    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true
recording_dir = "{dir}/queue-recordings"

[queue.queues.support]
name = "support"
extension = "7000"
agents = ["1002"]
record_calls = true
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let prompt = tone_prompt(&stack);
    announce_recordings(&stack, &prompt, ConsentFailureAction::Record, None).await;
    let (_alice, alice_phone, _bob, bob_phone) = two_phones(&stack).await;
    let (code, login) = stack.post("/agents/1002/login", "{}").await;
    assert_eq!(code, 200, "{login}");
    assert_eq!(login["status"], "logged_in", "{login}");

    // Alice calls the queue; Bob, its agent, takes the call.
    let alice_call = alice_phone.call("7000").await.unwrap();
    alice_call.answered(SOON).await.unwrap();
    let Some(incoming) = bob_phone.incoming(Duration::from_secs(10)).await else {
        stack.fail("the agent was not rung", &alice_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();

    let (alice_heard, bob_heard) = tokio::join!(
        alice_call.media().heard(PROMPT_HZ, SOON),
        bob_call.media().heard(PROMPT_HZ, SOON)
    );
    if !(alice_heard && bob_heard) {
        let why = format!(
            "announcement heard by the caller {alice_heard} ({} packets, {} frames of it), by the agent {bob_heard} ({} packets, {} frames of it)",
            alice_call.media().packets_received(),
            alice_call.media().tone_frames(PROMPT_HZ),
            bob_call.media().packets_received(),
            bob_call.media().tone_frames(PROMPT_HZ),
        );
        stack.fail(&why, &alice_phone).await;
    }
    let Some(file) = recording_started(&stack, SOON).await else {
        stack.fail("the queue did not record", &alice_phone).await;
    };

    // They talk over each other: the recording has both at once. Both tones
    // run past the hangup, so wherever the recorded window falls it is inside
    // the overlap rather than beside it, and they overlap for seconds so that
    // a loaded runner's window is still full of it.
    alice_call.media().play_tone(440.0, 12000);
    tokio::time::sleep(Duration::from_millis(400)).await;
    bob_call.media().play_tone(880.0, 12000);
    assert!(bob_call.media().heard(440, SOON).await);
    assert!(alice_call.media().heard(880, SOON).await);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);

    let records = stack.cdrs_at("/cdrs?kind=all", 2, SOON).await;
    let queue_record = records
        .iter()
        .find(|r| r["kind"] == "queue")
        .expect("queue record");
    assert_eq!(
        queue_record["recording_consent"], "announced",
        "{queue_record}"
    );
    assert_eq!(queue_record["recording_url"], file.display().to_string());

    let (_, samples) = fcp_testkit::wav::read_pcm(&file).unwrap();
    assert!(!fcp_testkit::wav::contains_tone(&samples, PROMPT_HZ));
    let (mut together, mut caller_alone, mut agent_alone, mut quiet) = (0, 0, 0, 0);
    for frame in samples.chunks(160) {
        let low = fcp_testkit::media::goertzel_ratio(frame, 440);
        let high = fcp_testkit::media::goertzel_ratio(frame, 880);
        // One tone on its own reads about 0.87 (the ratio leaks into
        // neighbouring bins), so two mixed tones can never sum to much
        // more than that: 0.7 measured 0.71 here and nothing on CI.
        match (low > 0.2, high > 0.2) {
            (true, true) if low + high > 0.55 => together += 1,
            (true, false) => caller_alone += 1,
            (false, true) => agent_alone += 1,
            (false, false) => quiet += 1,
            _ => {}
        }
    }
    // Which frames a failure has says what went wrong: the caller's or the
    // agent's audio missing from the mix, or the two written one after the
    // other rather than over each other.
    assert!(
        together >= 10,
        "the caller and the agent overlap in {together} frames of {}:          {caller_alone} have the caller alone, {agent_alone} the agent alone, {quiet} neither",
        samples.len() / 160
    );
    stack.finish().await;
}

/// A flow's Queue node puts the caller in the queue: the agent rings and
/// they talk. It set variables nobody read and went on at once, so the D8
/// menu's "press 2 for the queue" said goodbye (`docs/DEEP_TESTING.md` D8).
/// A queue that does not exist takes the node's error branch.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flows_queue_node_puts_the_caller_in_the_queue() {
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true

[queue.queues.support]
name = "support"
extension = "7000"
agents = ["1002"]
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let (_alice, alice_phone, _bob, bob_phone) = two_phones(&stack).await;
    let (code, login) = stack.post("/agents/1002/login", "{}").await;
    assert_eq!(code, 200, "{login}");

    // 1 for the queue, 2 for one that is not there, whose error branch
    // hangs up (its answer branch would go back to the menu).
    let menu = stack.seed.user("menu", "menu-pw", "1500").await;
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "queue-menu", "name": "Menu", "version": 1, "initial_node": "answer",
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "menu"},
            "menu": {
                "id": "menu", "name": "Menu", "type": "gather", "num_digits": 1,
                "timeout": 20, "transitions": {"1": "support", "2": "missing"},
                "default": "menu", "on_timeout": "menu"
            },
            "support": {
                "id": "support", "name": "Support", "type": "queue", "queue_name": "support",
                "next_on_answer": "done", "next_on_error": "menu"
            },
            "missing": {
                "id": "missing", "name": "Missing", "type": "queue", "queue_name": "nobody",
                "next_on_answer": "menu", "next_on_error": "done"
            },
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    let storage = stack.seed.storage();
    fcp_flow::FlowStore::insert_flow(storage, flow)
        .await
        .expect("seed flow");
    let menu_id = fcp_core::storage::CredentialStore::get_user_in_tenant(
        storage,
        &menu.username,
        &stack.seed.tenant_id,
    )
    .await
    .unwrap()
    .unwrap()
    .id;
    let mut settings = fcp_core::CallingSettings::defaults(stack.seed.tenant_id, menu_id);
    settings.forward_all = Some(fcp_core::CallTarget::Flow("queue-menu".into()));
    fcp_core::CallingStore::put_calling_settings(storage, settings)
        .await
        .unwrap();

    let alice_call = alice_phone.call(&menu.extension).await.unwrap();
    alice_call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    alice_call.media().send_dtmf('1');
    let Some(incoming) = bob_phone.incoming(Duration::from_secs(10)).await else {
        stack
            .fail("the Queue node did not ring the agent", &alice_phone)
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    alice_call.media().play_tone(440.0, 3000);
    bob_call.media().play_tone(880.0, 3000);
    assert!(
        bob_call.media().heard(440, SOON).await,
        "the agent does not hear the caller"
    );
    assert!(
        alice_call.media().heard(880, SOON).await,
        "the caller does not hear the agent"
    );
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(
        bob_call.ended(SOON).await,
        "the agent's leg outlived the call"
    );

    let lost = alice_phone.call(&menu.extension).await.unwrap();
    lost.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    lost.media().send_dtmf('2');
    assert!(
        lost.ended(SOON).await,
        "a missing queue did not take the error branch"
    );
    stack.finish().await;
}

/// A queue with no audio of its own plays the node's hold music, and an
/// agent the queue lists by extension logs in with `*50` from a phone that
/// dials by name. On the D8 server the caller waited two minutes in
/// silence, and the agent's `*50` would have been refused: the queue said
/// `1001`, the phone said `alice` (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_waiting_caller_hears_music_until_an_agent_logs_in_by_name() {
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true

[queue.queues.support]
name = "support"
extension = "7000"
agents = ["1002"]
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let (_alice, alice_phone, _bob, bob_phone) = two_phones(&stack).await;

    let alice_call = alice_phone.call("7000").await.unwrap();
    alice_call.answered(SOON).await.unwrap();
    assert!(
        alice_call.media().heard(DEFAULT_MUSIC_HZ, SOON).await,
        "the waiting caller hears no music"
    );

    let login = bob_phone.call("*50").await.unwrap();
    if let Err(e) = login.answered(SOON).await {
        stack
            .fail(&format!("the agent's *50 was refused: {e}"), &bob_phone)
            .await;
    }
    // Told, then hung up on: a bare 200 left the phone on a call (D8).
    assert!(
        login.ended(Duration::from_secs(15)).await,
        "the *50 call was never ended"
    );
    let Some(incoming) = bob_phone.incoming(Duration::from_secs(10)).await else {
        stack
            .fail("the logged-in agent was not rung", &alice_phone)
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    bob_call.media().play_tone(880.0, 3000);
    if !alice_call.media().heard(880, SOON).await {
        let why = format!(
            "the caller does not hear the agent ({} packets, {} frames of the tone; \
             the agent's leg received {} packets)",
            alice_call.media().packets_received(),
            alice_call.media().tone_frames(880),
            bob_call.media().packets_received(),
        );
        stack.fail(&why, &alice_phone).await;
    }
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert!(
        bob_call.ended(SOON).await,
        "the agent's leg outlived the call"
    );
    stack.finish().await;
}

/// The agent hangs up first: the caller is hung up on, and the agent,
/// after wrap-up, takes the next caller. On the D8 server the caller
/// stayed on the line, and the queue never let its agent off the call
/// (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_hanging_up_ends_the_callers_call_and_takes_the_next() {
    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true
wrap_up_time_secs = 1

[queue.queues.support]
name = "support"
extension = "7000"
agents = ["1002"]
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let (_alice, alice_phone, _bob, bob_phone) = two_phones(&stack).await;
    let login = bob_phone.call("*50").await.unwrap();
    login.answered(SOON).await.unwrap();
    assert!(login.ended(Duration::from_secs(15)).await);

    for round in ["first", "second"] {
        let alice_call = alice_phone.call("7000").await.unwrap();
        alice_call.answered(SOON).await.unwrap();
        let Some(incoming) = bob_phone.incoming(Duration::from_secs(10)).await else {
            stack
                .fail(
                    &format!("the agent was not rung for the {round} caller"),
                    &bob_phone,
                )
                .await;
        };
        let bob_call = incoming.answer().await.unwrap();
        bob_call.media().play_tone(880.0, 3000);
        assert!(
            alice_call.media().heard(880, SOON).await,
            "the {round} caller does not hear the agent"
        );
        assert_eq!(bob_call.hangup().await.unwrap(), 200);
        if !alice_call.ended(SOON).await {
            stack
                .fail(
                    &format!("the {round} caller stayed on after the agent hung up"),
                    &alice_phone,
                )
                .await;
        }
        // Wrap-up, then available again.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    stack.finish().await;
}

/// Between the agent's answer and the legs connected (the announcement a
/// recorded queue plays first) either party may hang up. The agent's
/// dialog was the queue's only once they were connected: the agent's BYE
/// there was refused 481 and the caller left talking to no one, and the
/// caller's left the agent on the line, on a call for good
/// (`docs/DEEP_TESTING.md` D8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hangup_during_the_announcement_ends_both_parties() {
    use fcp_core::compliance::ConsentFailureAction;

    let Some(stack) = Stack::start_with(
        r#"
[queue]
enabled = true
wrap_up_time_secs = 1
recording_dir = "{dir}/queue-recordings"

[queue.queues.support]
name = "support"
extension = "7000"
agents = ["1002"]
record_calls = true
"#,
        200,
    )
    .await
    else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let prompt = stack.dir().join("long-consent.wav");
    fcp_testkit::wav::write_tone(&prompt, PROMPT_HZ as f32, 6000).unwrap();
    announce_recordings(&stack, &prompt, ConsentFailureAction::Record, None).await;
    let (_alice, alice_phone, _bob, bob_phone) = two_phones(&stack).await;
    let (code, login) = stack.post("/agents/1002/login", "{}").await;
    assert_eq!(code, 200, "{login}");

    for agent_leaves in [true, false] {
        let who = if agent_leaves { "agent" } else { "caller" };
        let alice_call = alice_phone.call("7000").await.unwrap();
        alice_call.answered(SOON).await.unwrap();
        let Some(incoming) = bob_phone.incoming(Duration::from_secs(10)).await else {
            stack
                .fail(&format!("the agent was not rung ({who} round)"), &bob_phone)
                .await;
        };
        let bob_call = incoming.answer().await.unwrap();
        // Inside the announcement: both are hearing it.
        assert!(
            bob_call.media().heard(PROMPT_HZ, SOON).await,
            "the agent never heard the announcement ({who} round)"
        );
        let (leaving, staying, staying_phone) = if agent_leaves {
            (&bob_call, &alice_call, &alice_phone)
        } else {
            (&alice_call, &bob_call, &bob_phone)
        };
        assert_eq!(
            leaving.hangup().await.unwrap(),
            200,
            "the {who}'s BYE during the announcement"
        );
        if !staying.ended(SOON).await {
            stack
                .fail(
                    &format!("the other party stayed on after the {who} hung up"),
                    staying_phone,
                )
                .await;
        }
        // Wrap-up, then the agent is rung for the next caller.
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    stack.finish().await;
}

/// The built-in hold music's first note (`fcp_sip::hold_music`).
const DEFAULT_MUSIC_HZ: u32 = 659;

/// A registration whose phone is gone — crashed, or brought back by a
/// restore — is one of the contacts a call to the user forks to. The
/// live phone answers; the caller must hear that answer at once, not
/// after the dead leg's transactions give up (found by the nightly
/// stack, `docs/NIGHTLY_STACK.md` §4). The dead contact here is a port
/// that swallows everything and answers nothing, as a hung phone does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_registration_does_not_delay_the_answer() {
    use fcp_core::storage::{Registration, RegistrationStore};
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;

    // Bob's other phone: a binding in the store whose contact is a socket
    // nobody reads — no answer, no refusal, as a phone that hung or a
    // restore's stale row. Planted before Bob's live phone registers, so
    // the registrar's cache of his AOR is refreshed by that REGISTER.
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let dead_port = silent.local_addr().unwrap().port();
    let now = std::time::SystemTime::now();
    stack
        .seed
        .storage()
        .put_registration(Registration {
            tenant_id: stack.seed.tenant_id,
            aor: format!("sip:{}@{REALM}", bob.username),
            contact: format!("sip:{}@127.0.0.1:{dead_port}", bob.username),
            expires_at: now + Duration::from_secs(3600),
            call_id: format!("{}@dead", uuid::Uuid::new_v4()),
            cseq: 1,
            user_agent: Some("hung phone".to_string()),
            q: None,
            registered_at: now,
            path: None,
            flow_token: None,
            source_address: Some(format!("127.0.0.1:{dead_port}")),
            instance_id: None,
            reg_id: None,
            push: None,
            flow_node: None,
        })
        .await
        .unwrap();

    let alice_phone = stack.phone(&alice).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let started = tokio::time::Instant::now();
    let call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack
            .fail("Bob's live phone did not ring", &alice_phone)
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    if let Err(e) = call.answered(SOON).await {
        stack
            .fail(
                &format!("the answer did not reach the caller in time: {e}"),
                &alice_phone,
            )
            .await;
    }
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the answer took {:?}",
        started.elapsed()
    );
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);
    drop(silent);
    stack.finish().await;
}

/// A softphone is configured with the node's address and dials
/// `sip:2002@<address>`, which names no tenant; the caller's registration
/// does. Found by the nightly stack (`docs/NIGHTLY_STACK.md` §4): once a
/// tenant other than the node's default existed, such a call was
/// refused for want of a tenant, though the caller was registered and
/// digest-checked in one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_registered_phone_in_another_tenant_calls_by_the_node_address() {
    let stack = stack_or_skip!();
    let globex = stack.seed.tenant("globex", "globex.e2e.local").await;
    let alice = stack
        .seed
        .user_in(globex, "alice-globex", "alice-pw", "2001")
        .await;
    let bob = stack
        .seed
        .user_in(globex, "bob-globex", "bob-pw", "2002")
        .await;
    let alice_phone = Phone::new(
        &alice.username,
        &alice.password,
        "globex.e2e.local",
        stack.sip,
    )
    .await
    .unwrap();
    let bob_phone = Phone::new(&bob.username, &bob.password, "globex.e2e.local", stack.sip)
        .await
        .unwrap();
    assert_eq!(alice_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    // Dialled at the node's address, not at the tenant's domain.
    let call = alice_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack
            .fail(
                "Bob's phone did not ring: the call was not placed in the caller's tenant",
                &alice_phone,
            )
            .await;
    };
    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_number_that_records_its_calls_records_them_without_an_announcement_when_none_is_asked() {
    use fcp_core::storage::PhoneNumberStore;

    let stack = stack_or_skip!();
    // The caller's own number records its calls; the tenant asks for no
    // announcement.
    let caller = stack.seed.user("+15550001001", "caller-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let mut number =
        fcp_core::PhoneNumber::new("+15550001001".to_string(), stack.seed.tenant_id).unwrap();
    number.recording_enabled = true;
    stack.seed.storage().put_phone_number(number).await.unwrap();
    let caller_phone = stack.phone(&caller).await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(caller_phone.register().await.unwrap(), 200);
    assert_eq!(bob_phone.register().await.unwrap(), 200);

    let call = caller_phone.call(&bob.extension).await.unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &caller_phone).await;
    };
    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    let Some(file) = recording_started(&stack, SOON).await else {
        stack
            .fail("the number's call was not recorded", &caller_phone)
            .await;
    };
    call.media().play_tone(440.0, 1000);
    assert!(bob_call.media().heard(440, SOON).await);
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(bob_call.ended(SOON).await);

    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert_eq!(cdr["recording_consent"], "not_required", "{cdr}");
    assert_eq!(cdr["recording_url"], file.display().to_string());
    let rows = recording_rows(&stack, &cdr).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows[0].source,
        fcp_core::recording::CallRecordingSource::Number
    );
    assert_eq!(rows[0].consent, fcp_core::RecordingConsent::NotRequired);
    stack.finish().await;
}

/// A flow that answers and then hangs up: the node ends the call it
/// answered itself. That BYE goes out from inside the flow executor's own
/// chain, deep enough that it overflowed the worker's stack and aborted
/// the whole node — the call was never ended and its record never written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flow_that_hangs_up_ends_the_call_it_answered() {
    use fcp_core::storage::{LineBindingStore, PhoneNumberStore};

    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "answer-then-hangup",
        "name": "Answer then hang up",
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
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    let number = fcp_core::PhoneNumber::new("+15125550777".to_string(), stack.seed.tenant_id)
        .expect("valid number");
    let mut binding = fcp_core::LineBinding::new(
        number.id,
        None,
        fcp_core::BindingType::Ivr,
        stack.seed.tenant_id,
    );
    binding.flow_id = Some("answer-then-hangup".to_string());
    stack
        .seed
        .storage()
        .put_phone_number(number)
        .await
        .expect("seed number");
    stack
        .seed
        .storage()
        .put_line_binding(binding)
        .await
        .expect("seed binding");

    let phone = stack.phone(&alice).await;
    let call = phone.call("+15125550777").await.unwrap();
    call.answered(SOON).await.unwrap();
    assert!(call.ended(SOON).await, "the flow never hung the call up");
    let cdr = stack.cdrs(1, SOON).await.remove(0);
    assert!(
        cdr["to_uri"]
            .as_str()
            .is_some_and(|to| to.contains("+15125550777")),
        "the record is this call's: {cdr}"
    );
    // A flow answering is an answer: the record says so, and it has the
    // time and the duration that go with one.
    assert_eq!(cdr["disposition"], "Answered", "{cdr}");
    assert!(!cdr["answer_time_epoch"].is_null(), "{cdr}");
    assert!(!cdr["call_duration_ms"].is_null(), "{cdr}");
    stack.finish().await;
}

/// An INVITE challenged over UDP that rings a while is one call (RFC 3261
/// §17.2.1). The ACK to the 401 is the INVITE server transaction's; the
/// node passed every ACK to the B2BUA instead, so the 401 was resent for
/// half a minute, and a client that answers each resend with another
/// authenticated INVITE (siphon-rs's auto-retry did) placed the call again
/// and again — found by Anvil watching a busy lamp.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_challenged_call_that_rings_a_while_is_one_call() {
    let Some(stack) = Stack::start_registrar("auth_invites = true", "", "", 40).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    let call = alice_phone.call(&bob.extension).await.unwrap();
    // Ringing for longer than the 401's first resends (0.5 s, 1 s, 2 s).
    let mut rings = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(3500);
    while tokio::time::Instant::now() < deadline {
        if bob_phone
            .incoming(Duration::from_millis(250))
            .await
            .is_some()
        {
            rings += 1;
        }
    }
    assert_eq!(rings, 1, "Bob's phone rang {rings} times for one call");
    let _ = call.hangup().await;
    stack.finish().await;
}

/// An encrypted caller (SDES, RFC 4568) is answered encrypted, with the
/// node's own key, and its key stays on its own leg: the callee is offered
/// plain RTP and never sees it. The node used to pass the caller's
/// `RTP/SAVP` and `a=crypto` on to the callee, and answered the caller in
/// the clear.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_encrypted_caller_is_answered_encrypted_and_keeps_its_key() {
    let stack = stack_or_skip!();
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let bob_phone = stack.phone(&bob).await;
    assert_eq!(bob_phone.register().await.unwrap(), 200);
    let alice_phone = stack.phone(&alice).await;
    assert_eq!(alice_phone.register().await.unwrap(), 200);

    let key = "WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz";
    let call = alice_phone
        .call_offering_srtp(
            &bob.extension,
            &format!("1 AES_CM_128_HMAC_SHA1_80 inline:{key}"),
        )
        .await
        .unwrap();
    let Some(incoming) = bob_phone.incoming(SOON).await else {
        stack.fail("Bob's phone did not ring", &alice_phone).await;
    };
    let offer = String::from_utf8_lossy(incoming.body()).to_string();
    assert!(
        offer.contains(" RTP/AVP "),
        "Bob is offered plain RTP: {offer}"
    );
    assert!(!offer.contains("a=crypto"), "Bob was handed a key: {offer}");
    assert!(!offer.contains(key), "Bob was handed Alice's key: {offer}");

    let bob_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.expect("answered");
    let answer = call.answer_sdp().expect("Alice's answer");
    assert!(
        answer.contains(" RTP/SAVP "),
        "Alice is answered encrypted: {answer}"
    );
    let crypto = answer
        .lines()
        .find_map(|l| l.strip_prefix("a=crypto:"))
        .unwrap_or_else(|| panic!("Alice's answer has no key: {answer}"));
    assert!(
        crypto.starts_with("1 AES_CM_128_HMAC_SHA1_80 inline:"),
        "the answer keeps the offer's tag and suite: {crypto}"
    );
    assert!(
        !crypto.contains(key),
        "the node answered with Alice's own key"
    );

    call.hangup().await.unwrap();
    assert!(bob_call.ended(SOON).await);
    stack.finish().await;
}
