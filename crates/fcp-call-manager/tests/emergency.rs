//! Emergency calls through the call manager over real SIP
//! (`docs/E911_BLOCK_2.md` §10): a registered phone dials 911 and a
//! scripted phone standing in for the carrier's emergency trunk receives
//! the site's callback number, `P-Asserted-Identity` and `Priority:
//! emergency`; a test call is recorded as one; a failing emergency trunk
//! gives way to the next; a flow's `Dial` node is an emergency path; the
//! caller a trusted edge names is believed only from a trusted edge; and
//! every record is completed when its call ends. Skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_core::e911::{EmergencyCallRecord, EmergencyKind};
use fcp_core::storage::{LineBindingStore, PhoneNumberStore};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::{http_get, loopback};
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(10);
const ALICE_ELIN: &str = "+15185550100";
const BOB_ELIN: &str = "+15185550199";
const BRANCH_ELIN: &str = "+15185550222";

/// A running call manager whose emergency trunks are scripted phones.
struct Stack {
    service: Option<Service>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    http: u16,
    _dir: tempfile::TempDir,
}

/// A carrier's emergency trunk: a scripted phone the call manager routes
/// to by address.
struct Carrier {
    name: &'static str,
    priority: u32,
    /// `emergency_location`, as TOML.
    location: &'static str,
    phone: Phone,
}

impl Carrier {
    async fn new(name: &'static str, priority: u32) -> Self {
        Self::taking(name, priority, r#"["elin"]"#).await
    }

    /// A carrier whose trunk takes the location as `location` says
    /// (`["pidf_lo"]`, `["elin", "pidf_lo"]`).
    async fn taking(name: &'static str, priority: u32, location: &'static str) -> Self {
        // The phone never sends to its "server"; it only answers.
        let nowhere = SocketAddr::from(([127, 0, 0, 1], 9));
        let phone = Phone::new(name, "x", REALM, nowhere)
            .await
            .expect("carrier phone binds");
        Self {
            name,
            priority,
            location,
            phone,
        }
    }

    /// Where this carrier answers, for a trunk created through the store
    /// rather than written into the node's TOML.
    fn port(&self) -> u16 {
        self.phone.local_addr.port()
    }

    fn trunk_toml(&self) -> String {
        format!(
            r#"
[[trunks]]
name = "{name}"
enabled = true
direction = "Upstream"
hosts = ["127.0.0.1"]
weight = 100
priority = {priority}
register = false
max_concurrent_calls = 0
emergency_capable = true
emergency_location = {location}

[trunks.transport]
protocol = "Udp"
port = {port}
verify_tls = false

[trunks.failover]
max_failures = 3
recovery_interval_secs = 30
health_check_enabled = false
health_check_interval_secs = 60
"#,
            name = self.name,
            priority = self.priority,
            location = self.location,
            port = self.phone.local_addr.port(),
        )
    }
}

impl Stack {
    /// `None` without a database. `trusted_edges` goes into `[e911]`.
    async fn start(carriers: &[&Carrier], trusted_edges: &[&str]) -> Option<Self> {
        Self::start_with_events(carriers, trusted_edges, None).await
    }

    /// As [`Stack::start`], forwarding events to `ingest` (an events
    /// server's `/ingest`), which is how the notifier hears about an
    /// emergency call (`docs/E911_BLOCK_2.md` §8.3).
    async fn start_with_events(
        carriers: &[&Carrier],
        trusted_edges: &[&str],
        ingest: Option<&str>,
    ) -> Option<Self> {
        Self::start_speaking(carriers, trusted_edges, ingest, None).await
    }

    /// As [`Stack::start_with_events`], speaking through the TTS service
    /// at `tts` (`[media] tts_remote_url`).
    async fn start_speaking(
        carriers: &[&Carrier],
        trusted_edges: &[&str],
        ingest: Option<&str>,
        tts: Option<&str>,
    ) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let edges = trusted_edges
            .iter()
            .map(|e| format!("\"{e}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let trunks: String = carriers.iter().map(|c| c.trunk_toml()).collect();
        let config = format!(
            r#"
[service]
node_id = "e2e-emergency"

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

[b2bua.forking]
timeout = 5

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/prompts"
tts_cache_dir = "{dir}/tts-cache"
recording_dir = "{dir}/recordings"
{tts}

[logging]
level = "info"
format = "compact"

# A trunk a tenant creates while the node runs reaches it on the backstop
# read, turned right down so a test does not wait a minute for it.
[trunk_routing]
refresh_secs = 1

[e911]
trusted_edges = [{edges}]
{events}
{trunks}
"#,
            db = db.url(),
            rtp_end = rtp_start + 60,
            dir = dir.path().display(),
            tts = match tts {
                Some(url) =>
                    format!("tts_remote_url = \"{url}\"\ntts_remote_token = \"tts-token\""),
                None => String::new(),
            },
            events = match ingest {
                Some(url) => format!("\n[events]\nserver_url = \"{url}\"\n"),
                None => String::new(),
            },
        );
        let service =
            Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
        Some(Self {
            service: Some(service),
            db: Some(db),
            seed,
            sip: SocketAddr::from(([127, 0, 0, 1], sip_port)),
            http,
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

    /// The test database everything in this stack shares.
    fn db_url(&self) -> String {
        self.db.as_ref().expect("a database").url().to_string()
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(self.http, path))
            .await
            .unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(code, 200, "GET {path}: {body}");
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("GET {path}: {e}: {body}"))
    }

    /// Give up with the service's logs.
    async fn fail(&mut self, why: &str) -> ! {
        let logs = match self.service.take() {
            Some(service) => service.kill().await,
            None => String::new(),
        };
        panic!("{why}\n{logs}");
    }

    async fn finish(mut self) {
        if let Some(service) = self.service.take() {
            service.stop().await;
        }
        if let Some(db) = self.db.take() {
            db.drop_db().await;
        }
    }
}

macro_rules! stack_or_skip {
    ($carriers:expr, $edges:expr) => {
        match Stack::start($carriers, $edges).await {
            Some(s) => s,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// The record for the call placed to `dialed`, once it has ended.
fn ended(records: &[EmergencyCallRecord], dialed: &str) -> EmergencyCallRecord {
    records
        .iter()
        .find(|r| r.dialed_number == dialed)
        .cloned()
        .unwrap_or_else(|| panic!("no record for {dialed}: {records:#?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_911_call_reaches_the_emergency_trunk_as_the_sites_callback_number() {
    let carrier = Carrier::new("psap-primary", 1).await;
    let mut stack = stack_or_skip!(&[&carrier], &[]);
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    let alice_phone = stack.phone(&alice).await;

    // The node lists the trunk it routes emergency calls to.
    let trunks = stack.get("/emergency/trunks").await;
    assert_eq!(trunks["trunks"][0]["name"], "psap-primary", "{trunks}");
    assert_eq!(trunks["routable"], true, "{trunks}");

    // Alice dials 911: the carrier's INVITE carries her site's ELIN.
    let call = alice_phone.call("911").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!(
            "the emergency trunk never rang; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    assert!(
        incoming.request_uri().starts_with("sip:911@127.0.0.1"),
        "{}",
        incoming.request_uri()
    );
    assert!(
        incoming.from.contains(ALICE_ELIN),
        "From: {}",
        incoming.from
    );
    assert_eq!(
        incoming.header("P-Asserted-Identity").as_deref(),
        Some(format!("<sip:{ALICE_ELIN}@{REALM}>").as_str())
    );
    assert_eq!(incoming.header("Priority").as_deref(), Some("emergency"));
    assert!(
        incoming.multipart().is_none() && incoming.header("Geolocation").is_none(),
        "an ELIN trunk gets no location by value"
    );
    let carrier_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(carrier_call.ended(SOON).await, "the carrier's leg ends");

    // A 933 test call goes the same way and is recorded as a test.
    let test_call = alice_phone.call("933").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the test call never rang; it is {:?}", test_call.phase());
        stack.fail(&why).await;
    };
    assert_eq!(incoming.header("Priority").as_deref(), Some("emergency"));
    incoming.reject(486, "Busy Here").await.unwrap();
    let refused = test_call.answered(SOON).await.unwrap_err().to_string();
    assert!(
        refused.contains("failed"),
        "the refused test call fails: {refused}"
    );

    // Both records are completed with what became of the call.
    let records = stack
        .seed
        .emergency_calls(2, SOON, |r| r.ended_at.is_some())
        .await;
    let live = ended(&records, "911");
    assert_eq!(live.kind, EmergencyKind::Live);
    assert_eq!(live.callback_number.as_deref(), Some(ALICE_ELIN));
    assert_eq!(live.caller_username.as_deref(), Some("alice"));
    assert_eq!(live.trunk_name.as_deref(), Some("psap-primary"));
    assert_eq!(live.outcome, "answered", "{live:#?}");
    assert_eq!(live.sip_status, Some(200));
    assert_eq!(live.delivery.as_deref(), Some("elin"));
    let answered = live.answered_at.expect("answered_at");
    let duration = live.ended_at.unwrap().duration_since(answered).unwrap();
    assert!(duration >= Duration::from_secs(1), "{duration:?}");
    let test = ended(&records, "933");
    assert_eq!(test.kind, EmergencyKind::Test);
    assert_eq!(test.outcome, "busy", "{test:#?}");
    assert_eq!(test.sip_status, Some(486));
    assert!(test.answered_at.is_none());

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_emergency_trunk_gives_way_to_the_next() {
    let primary = Carrier::new("psap-primary", 1).await;
    let backup = Carrier::new("psap-backup", 2).await;
    let mut stack = stack_or_skip!(&[&primary, &backup], &[]);
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    let alice_phone = stack.phone(&alice).await;

    let call = alice_phone.call("911").await.unwrap();
    let Some(incoming) = primary.phone.incoming(SOON).await else {
        let why = format!(
            "the primary trunk never rang; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    // One carrier at a time: the backup does not ring while the primary
    // still might answer.
    assert!(
        backup
            .phone
            .incoming(Duration::from_millis(1500))
            .await
            .is_none(),
        "the backup trunk rang alongside the primary"
    );
    incoming.reject(503, "Service Unavailable").await.unwrap();
    let Some(incoming) = backup.phone.incoming(SOON).await else {
        let why = format!(
            "the backup trunk never rang; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    assert!(
        incoming.from.contains(ALICE_ELIN),
        "From: {}",
        incoming.from
    );
    let carrier_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(carrier_call.ended(SOON).await);

    // The record names the trunk that answered.
    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    assert_eq!(records[0].trunk_name.as_deref(), Some("psap-backup"));
    assert_eq!(records[0].outcome, "answered", "{:#?}", records[0]);

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flow_dialing_911_places_an_emergency_call() {
    let carrier = Carrier::taking("psap-primary", 1, r#"["elin", "pidf_lo"]"#).await;
    let mut stack = stack_or_skip!(&[&carrier], &[]);
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    let alice_phone = stack.phone(&alice).await;

    // A number whose flow answers and dials 911.
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": "dial-911",
        "name": "Dial 911",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": stack.seed.tenant_id.to_string(),
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "dial"},
            "dial": {
                "id": "dial", "name": "Dial 911", "type": "dial",
                "destination": "911", "timeout": 10, "next": "done",
                "on_error": "done", "on_no_answer": "done", "on_busy": "done"
            },
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    let number = fcp_core::PhoneNumber::new("+15185551000".to_string(), stack.seed.tenant_id)
        .expect("valid number");
    let mut binding = fcp_core::LineBinding::new(
        number.id,
        None,
        fcp_core::BindingType::Ivr,
        stack.seed.tenant_id,
    );
    binding.flow_id = Some("dial-911".to_string());
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

    let call = alice_phone.call("+15185551000").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!(
            "the flow never dialed the trunk; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    assert!(
        incoming.request_uri().starts_with("sip:911@127.0.0.1"),
        "{}",
        incoming.request_uri()
    );
    assert!(
        incoming.from.contains(ALICE_ELIN),
        "From: {}",
        incoming.from
    );
    assert_eq!(incoming.header("Priority").as_deref(), Some("emergency"));
    let body = incoming
        .multipart()
        .expect("the flow's INVITE carries the location");
    let cid = incoming.header("Geolocation").expect("Geolocation");
    assert!(body
        .part_by_content_id(cid.trim_start_matches('<').trim_end_matches('>'))
        .is_some());
    let carrier_call = incoming.answer().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    let _ = carrier_call.ended(SOON).await;

    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    let record = &records[0];
    assert_eq!(record.dialed_number, "911");
    assert_eq!(record.callback_number.as_deref(), Some(ALICE_ELIN));
    assert_eq!(record.trunk_name.as_deref(), Some("psap-primary"));
    assert_eq!(record.outcome, "answered", "{record:#?}");
    assert_eq!(record.sip_status, Some(200));
    assert!(record.answered_at.is_some());
    assert_eq!(record.delivery.as_deref(), Some("elin+pidf_lo"));

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_named_caller_is_believed_only_from_a_trusted_edge() {
    for trusted in [false, true] {
        let carrier = Carrier::new("psap-primary", 1).await;
        let edges: &[&str] = if trusted { &["127.0.0.1"] } else { &[] };
        let mut stack = stack_or_skip!(&[&carrier], edges);
        let alice = stack.seed.user("alice", "alice-pw", "1001").await;
        stack.seed.user("bob", "bob-pw", "1002").await;
        stack.seed.site_for("alice", ALICE_ELIN).await;
        stack.seed.site_for("bob", BOB_ELIN).await;
        let alice_phone = stack.phone(&alice).await;

        // Alice's phone names Bob as the caller.
        let tenant = stack.seed.tenant_id.to_string();
        let call = alice_phone
            .call_with_headers(
                "911",
                &[
                    (fcp_core::CALLER_TENANT_HEADER, tenant.as_str()),
                    (fcp_core::CALLER_USER_HEADER, "bob"),
                ],
            )
            .await
            .unwrap();
        let Some(incoming) = carrier.phone.incoming(SOON).await else {
            let why = format!("the trunk never rang; the call is {:?}", call.phase());
            stack.fail(&why).await;
        };
        let expected = if trusted { BOB_ELIN } else { ALICE_ELIN };
        assert!(
            incoming.from.contains(expected),
            "trusted={trusted}: From: {}",
            incoming.from
        );
        incoming.reject(486, "Busy Here").await.unwrap();
        assert!(call.answered(SOON).await.is_err(), "the refused call fails");
        let records = stack
            .seed
            .emergency_calls(1, SOON, |r| r.ended_at.is_some())
            .await;
        let caller = if trusted { "bob" } else { "alice" };
        assert_eq!(records[0].caller_username.as_deref(), Some(caller));

        stack.finish().await;
    }
}

/// Alice's site with its street in civic elements and on the map.
async fn structure_alice_site(seed: &Seed) {
    use fcp_core::storage::{CredentialStore, SiteLocationStore};
    let user = seed
        .storage()
        .get_user_in_tenant("alice", &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let mut site = seed
        .storage()
        .get_location(&user.location_id.unwrap())
        .await
        .unwrap()
        .unwrap();
    site.civic.house_number = Some("100".into());
    site.civic.street_name = Some("State".into());
    site.civic.street_suffix = Some("St".into());
    site.civic.room = Some("312".into());
    site.latitude = Some(42.6526);
    site.longitude = Some(-73.7562);
    seed.storage().put_location(site).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pidf_lo_trunk_receives_the_location_by_value() {
    let carrier = Carrier::taking("psap-ng911", 1, r#"["elin", "pidf_lo"]"#).await;
    let mut stack = stack_or_skip!(&[&carrier], &[]);
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    structure_alice_site(&stack.seed).await;
    let alice_phone = stack.phone(&alice).await;

    let call = alice_phone.call("911").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the NG911 trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };

    // RFC 6442: the SDP and the PIDF-LO in one body, the location named
    // by Content-ID from Geolocation, routing on it allowed; the callback
    // number still the caller.
    assert!(
        incoming.from.contains(ALICE_ELIN),
        "From: {}",
        incoming.from
    );
    let body = incoming.multipart().expect("a multipart body");
    assert!(body.part_by_type("application/sdp").is_some());
    let geolocation = incoming.header("Geolocation").expect("Geolocation");
    let cid = geolocation
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_string();
    assert!(cid.starts_with("cid:"), "{geolocation}");
    let pidf = body
        .part_by_content_id(&cid)
        .expect("the part Geolocation names");
    assert_eq!(pidf.media_type(), "application/pidf+xml");
    assert_eq!(
        incoming.header("Geolocation-Routing").as_deref(),
        Some("yes")
    );
    let xml = String::from_utf8_lossy(pidf.body()).to_string();
    for expected in [
        format!("entity=\"pres:{ALICE_ELIN}@{REALM}\""),
        "<ca:country>US</ca:country>".to_string(),
        "<ca:A1>NY</ca:A1>".to_string(),
        "<ca:A3>Albany</ca:A3>".to_string(),
        "<ca:RD>State</ca:RD>".to_string(),
        "<ca:STS>St</ca:STS>".to_string(),
        "<ca:HNO>100</ca:HNO>".to_string(),
        "<ca:FLR>3</ca:FLR>".to_string(),
        "<ca:PC>12207</ca:PC>".to_string(),
        "<ca:ROOM>312</ca:ROOM>".to_string(),
        "<gml:pos>42.6526 -73.7562</gml:pos>".to_string(),
        "<gbp:retransmission-allowed>true</gbp:retransmission-allowed>".to_string(),
        "<gp:method>Manual</gp:method>".to_string(),
    ] {
        assert!(xml.contains(&expected), "{expected} in {xml}");
    }
    let carrier_call = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(carrier_call.ended(SOON).await);

    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    assert_eq!(records[0].delivery.as_deref(), Some("elin+pidf_lo"));
    assert_eq!(records[0].location_structured, Some(true));
    assert_eq!(records[0].geolocation_error, None);

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_location_is_retried_as_elin_or_gives_way_to_the_next_trunk() {
    const REFUSAL: &str = "300;code=\"Insufficient Location Information\"";
    // A trunk that takes both, refusing the location: the same trunk again
    // with ELIN alone.
    {
        let carrier = Carrier::taking("psap-both", 1, r#"["elin", "pidf_lo"]"#).await;
        let mut stack = stack_or_skip!(&[&carrier], &[]);
        let alice = stack.seed.user("alice", "alice-pw", "1001").await;
        stack.seed.site_for("alice", ALICE_ELIN).await;
        let alice_phone = stack.phone(&alice).await;

        let call = alice_phone.call("911").await.unwrap();
        let Some(incoming) = carrier.phone.incoming(SOON).await else {
            let why = format!("the trunk never rang; the call is {:?}", call.phase());
            stack.fail(&why).await;
        };
        assert!(incoming.multipart().is_some());
        incoming
            .reject_with_headers(
                424,
                "Bad Location Information",
                &[("Geolocation-Error", REFUSAL)],
            )
            .await
            .unwrap();
        let Some(again) = carrier.phone.incoming(SOON).await else {
            let why = format!(
                "the trunk was not tried again; the call is {:?}",
                call.phase()
            );
            stack.fail(&why).await;
        };
        assert!(
            again.multipart().is_none(),
            "the retry carries no location by value"
        );
        assert!(again.header("Geolocation").is_none());
        assert!(again.from.contains(ALICE_ELIN), "From: {}", again.from);
        let carrier_call = again.answer().await.unwrap();
        call.answered(SOON).await.unwrap();
        assert_eq!(call.hangup().await.unwrap(), 200);
        assert!(carrier_call.ended(SOON).await);
        let records = stack
            .seed
            .emergency_calls(1, SOON, |r| r.ended_at.is_some())
            .await;
        assert_eq!(records[0].trunk_name.as_deref(), Some("psap-both"));
        assert_eq!(records[0].delivery.as_deref(), Some("elin"));
        assert_eq!(records[0].geolocation_error.as_deref(), Some(REFUSAL));
        assert_eq!(records[0].outcome, "answered");
        stack.finish().await;
    }

    // A trunk that takes only the location, refusing it: the next trunk.
    {
        let ng911 = Carrier::taking("psap-ng911", 1, r#"["pidf_lo"]"#).await;
        let backup = Carrier::new("psap-elin", 2).await;
        let mut stack = stack_or_skip!(&[&ng911, &backup], &[]);
        let alice = stack.seed.user("alice", "alice-pw", "1001").await;
        stack.seed.site_for("alice", ALICE_ELIN).await;
        let alice_phone = stack.phone(&alice).await;

        let call = alice_phone.call("911").await.unwrap();
        let Some(incoming) = ng911.phone.incoming(SOON).await else {
            let why = format!("the NG911 trunk never rang; the call is {:?}", call.phase());
            stack.fail(&why).await;
        };
        incoming
            .reject_with_headers(
                424,
                "Bad Location Information",
                &[("Geolocation-Error", REFUSAL)],
            )
            .await
            .unwrap();
        assert!(
            ng911
                .phone
                .incoming(Duration::from_millis(1500))
                .await
                .is_none(),
            "a pidf_lo-only trunk is not retried without the location"
        );
        let Some(incoming) = backup.phone.incoming(SOON).await else {
            let why = format!("the ELIN trunk never rang; the call is {:?}", call.phase());
            stack.fail(&why).await;
        };
        let carrier_call = incoming.answer().await.unwrap();
        call.answered(SOON).await.unwrap();
        assert_eq!(call.hangup().await.unwrap(), 200);
        assert!(carrier_call.ended(SOON).await);
        let records = stack
            .seed
            .emergency_calls(1, SOON, |r| r.ended_at.is_some())
            .await;
        assert_eq!(records[0].trunk_name.as_deref(), Some("psap-elin"));
        assert_eq!(records[0].delivery.as_deref(), Some("elin"));
        assert_eq!(records[0].geolocation_error.as_deref(), Some(REFUSAL));
        stack.finish().await;
    }
}

/// A second site, with its own ELIN, and a network that puts `cidr` at it.
async fn network_site(
    seed: &Seed,
    name: &str,
    elin: &str,
    cidr: &str,
    applies_to: fcp_core::e911::NetworkApplies,
) -> fcp_core::LocationId {
    use fcp_core::e911::EmergencyStore;
    use fcp_core::storage::SiteLocationStore;

    let mut site = fcp_core::SiteLocation::new(name.to_string(), seed.tenant_id);
    site.address_line1 = Some("9 Elm St".to_string());
    site.city = Some("Troy".to_string());
    site.state = Some("NY".to_string());
    site.postal_code = Some("12180".to_string());
    site.country = Some("US".to_string());
    site.e911_callback_number = Some(elin.to_string());
    let id = site.id;
    seed.storage().put_location(site).await.unwrap();

    let store =
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(seed.storage().pool().clone());
    let mut network =
        fcp_core::e911::EmergencyNetwork::new(seed.tenant_id, cidr.parse().unwrap(), id);
    network.applies_to = applies_to;
    network.name = Some(format!("{name} wired"));
    store.put_emergency_network(&network).await.unwrap();
    id
}

/// A user's device, as the seed made it.
async fn user_device(seed: &Seed, username: &str) -> fcp_core::Device {
    use fcp_core::storage::{CredentialStore, DeviceStore};
    let user = seed
        .storage()
        .get_user_in_tenant(username, &seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    seed.storage()
        .list_devices_by_user(&user.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("the seed gave {username} a device"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_is_placed_by_the_network_it_is_on() {
    use fcp_core::storage::DeviceStore;

    let carrier = Carrier::new("psap-primary", 1).await;
    let mut stack = stack_or_skip!(&[&carrier], &[]);
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    // 127.0.0.2 is the branch office; every 127.0.0.0/8 address is local.
    network_site(
        &stack.seed,
        "Branch",
        BRANCH_ELIN,
        "127.0.0.2/32",
        fcp_core::e911::NetworkApplies::Nomadic,
    )
    .await;

    // A softphone at the branch: the call carries the branch's ELIN, and
    // the record says the network placed it.
    let phone = Phone::bound_to(
        "127.0.0.2:0",
        &alice.username,
        &alice.password,
        REALM,
        stack.sip,
    )
    .await
    .unwrap();
    assert_eq!(phone.register().await.unwrap(), 200);
    let call = phone.call("911").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    assert!(
        incoming.from.contains(BRANCH_ELIN),
        "the branch's ELIN: {}",
        incoming.from
    );
    incoming.reject(486, "Busy Here").await.unwrap();
    assert!(call.answered(SOON).await.is_err());
    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    let location = records[0].location.as_ref().expect("a location");
    assert_eq!(location.source, fcp_core::e911::LocationSource::Network);
    assert_eq!(location.client_address.as_deref(), Some("127.0.0.2"));
    assert_eq!(location.location_name.as_deref(), Some("Branch"));

    // A desk phone keeps the site it was provisioned at: a network that
    // places only nomadic devices does not move it. (Bob's device is a desk
    // phone from the start: the node caches devices as it registers them.)
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    stack.seed.site_for("bob", BOB_ELIN).await;
    // A device's type is fixed when it is created, so bob gets a desk
    // phone in place of the seed's softphone.
    let softphone = user_device(&stack.seed, "bob").await;
    let mut desk_phone = fcp_core::Device::new(
        "bob's desk phone".to_string(),
        fcp_core::DeviceType::DeskPhone,
        stack.seed.tenant_id,
    );
    desk_phone.owner_user_id = softphone.owner_user_id;
    desk_phone.sip_uri = softphone.sip_uri.clone();
    stack
        .seed
        .storage()
        .delete_device(&softphone.id)
        .await
        .unwrap();
    stack.seed.storage().put_device(desk_phone).await.unwrap();
    let desk = Phone::bound_to(
        "127.0.0.2:0",
        &bob.username,
        &bob.password,
        REALM,
        stack.sip,
    )
    .await
    .unwrap();
    assert_eq!(desk.register().await.unwrap(), 200);
    let call = desk.call("911").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    assert!(
        incoming.from.contains(BOB_ELIN),
        "a desk phone keeps its site: {}",
        incoming.from
    );
    incoming.reject(486, "Busy Here").await.unwrap();
    assert!(call.answered(SOON).await.is_err());

    // Until the tenant says that network places everything on it.
    {
        use fcp_core::e911::EmergencyStore;
        let store = fcp_storage_postgres::emergency::PostgresEmergencyStore::new(
            stack.seed.storage().pool().clone(),
        );
        let mut network = store
            .list_emergency_networks(&stack.seed.tenant_id)
            .await
            .unwrap()
            .remove(0);
        network.applies_to = fcp_core::e911::NetworkApplies::All;
        store.put_emergency_network(&network).await.unwrap();
    }
    let call = desk.call("911").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    assert!(
        incoming.from.contains(BRANCH_ELIN),
        "applies_to = all places the desk phone too: {}",
        incoming.from
    );
    incoming.reject(486, "Busy Here").await.unwrap();
    assert!(call.answered(SOON).await.is_err());

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_declared_location_places_a_caller_nothing_else_places() {
    use fcp_core::e911::EmergencyStore;
    use fcp_core::storage::CredentialStore;

    let carrier = Carrier::new("psap-primary", 1).await;
    let mut stack = stack_or_skip!(&[&carrier], &[]);
    // Bob has no site and no network: only what he said about himself.
    let bob = stack.seed.user("bob", "bob-pw", "1002").await;
    let user = stack
        .seed
        .storage()
        .get_user_in_tenant("bob", &stack.seed.tenant_id)
        .await
        .unwrap()
        .unwrap();
    let store = fcp_storage_postgres::emergency::PostgresEmergencyStore::new(
        stack.seed.storage().pool().clone(),
    );
    let now = std::time::SystemTime::now();
    let mut address = fcp_core::E911Address::new(
        "500 Hotel Rd".to_string(),
        "Saratoga Springs".to_string(),
        "NY".to_string(),
        "12866".to_string(),
        "US".to_string(),
    );
    address.validated = true;
    store
        .put_declared_location(&fcp_core::e911::DeclaredLocation {
            user_id: user.id,
            tenant_id: stack.seed.tenant_id,
            address,
            declared_at: now - Duration::from_secs(120),
            expires_at: now + Duration::from_secs(3600),
        })
        .await
        .unwrap();

    let phone = stack.phone(&bob).await;
    let call = phone.call("911").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    incoming.reject(486, "Busy Here").await.unwrap();
    assert!(call.answered(SOON).await.is_err());

    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    let location = records[0].location.as_ref().expect("a location");
    assert_eq!(location.source, fcp_core::e911::LocationSource::Declared);
    assert!(location.address.address1.contains("Hotel"));
    assert!(
        location.declared_age.unwrap() >= Duration::from_secs(120),
        "the record dates the declaration: {:?}",
        location.declared_age
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_devices_own_location_is_used_only_from_a_trusted_device() {
    use fcp_core::storage::DeviceStore;

    let carrier = Carrier::taking("psap-ng911", 1, r#"["elin", "pidf_lo"]"#).await;
    let mut stack = stack_or_skip!(&[&carrier], &[]);
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    let phone = stack.phone(&alice).await;

    // The location the device sends: a PIDF-LO beside its SDP.
    let sent = fcp_core::pidf_lo::document(
        &fcp_core::e911::DispatchableLocation {
            address: fcp_core::E911Address::new(
                "12 Conference Way".to_string(),
                "Albany".to_string(),
                "NY".to_string(),
                "12207".to_string(),
                "US".to_string(),
            ),
            callback_number: None,
            source: fcp_core::e911::LocationSource::DeviceOverride,
            location_id: None,
            location_name: None,
            point: None,
            client_address: None,
            declared_age: None,
        },
        &fcp_core::pidf_lo::Subject {
            entity: "pres:alice@e2e.local",
            device_id: "urn:uuid:11111111-2222-3333-4444-555555555555",
            timestamp: std::time::SystemTime::now(),
        },
    );

    // An untrusted device's claim is ignored: the site stands.
    let call = phone
        .call_with_body_parts("911", &[(fcp_core::pidf_lo::CONTENT_TYPE, &sent)], &[])
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    let body = incoming
        .multipart()
        .expect("the location goes out by value");
    let pidf = body
        .part_by_type(fcp_core::pidf_lo::CONTENT_TYPE)
        .expect("a PIDF-LO part");
    let xml = String::from_utf8_lossy(pidf.body()).to_string();
    assert!(
        xml.contains("State St") && !xml.contains("Conference Way"),
        "an untrusted device's own location is not dispatched: {xml}"
    );
    incoming.reject(486, "Busy Here").await.unwrap();
    assert!(call.answered(SOON).await.is_err());

    // The tenant trusts this device: now its claim is what goes out.
    let mut device = user_device(&stack.seed, "alice").await;
    device.e911_trust_location = Some(true);
    stack.seed.storage().put_device(device).await.unwrap();
    let call = phone
        .call_with_body_parts("911", &[(fcp_core::pidf_lo::CONTENT_TYPE, &sent)], &[])
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    let body = incoming
        .multipart()
        .expect("the location goes out by value");
    let xml = String::from_utf8_lossy(
        body.part_by_type(fcp_core::pidf_lo::CONTENT_TYPE)
            .unwrap()
            .body(),
    )
    .to_string();
    assert!(xml.contains("Conference Way"), "{xml}");
    assert!(
        incoming.from.contains(ALICE_ELIN),
        "the callback number is still the site's: {}",
        incoming.from
    );
    incoming.reject(486, "Busy Here").await.unwrap();
    assert!(call.answered(SOON).await.is_err());

    let records = stack
        .seed
        .emergency_calls(2, SOON, |r| r.ended_at.is_some())
        .await;
    let sources: Vec<_> = records
        .iter()
        .map(|r| r.location.as_ref().unwrap().source)
        .collect();
    assert!(
        sources.contains(&fcp_core::e911::LocationSource::DeviceSupplied)
            && sources.contains(&fcp_core::e911::LocationSource::UserLocation),
        "{sources:?}"
    );
    stack.finish().await;
}

/// The chain Kari's Law asks for, end to end (`docs/E911_BLOCK_2.md` §8):
/// a `933` test call is placed, the events server hears `call.emergency`,
/// and the front desk's email arrives saying it was a test — with the
/// attempt recorded and the call's record marked notified.
#[tokio::test]
async fn a_test_call_tells_the_tenants_contacts() {
    use fcp_core::e911::notify::{ContactKind, NotifyContact};
    use fcp_core::e911::{EmergencyConfig, EmergencyStore};

    let carrier = Carrier::new("carrier-a", 10).await;
    let mail = fcp_testkit::SmtpSink::start().await;
    let events_port = free_port();
    let ingest = loopback(events_port, "/ingest");
    let mut stack = match Stack::start_with_events(&[&carrier], &[], Some(&ingest)).await {
        Some(s) => s,
        None => {
            eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
            return;
        }
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;

    // Who this tenant is told at.
    let store = fcp_storage_postgres::emergency::PostgresEmergencyStore::new(
        stack.seed.storage().pool().clone(),
    );
    let mut config = EmergencyConfig::defaults(stack.seed.tenant_id);
    config.contacts = vec![NotifyContact::new(
        ContactKind::Email,
        "frontdesk@example.com",
    )];
    store.put_emergency_config(&config).await.unwrap();

    // An events server with nothing but the email channel.
    let events_config = format!(
        r#"
[service]
node_id = "e2e-events"

[websocket]
bind_address = "127.0.0.1:{events_port}"

[event_bus]
bus_type = "memory"

[webhook]
enabled = false
database_url = "{db}"

[notifications]
enabled = true
max_attempts = 1
retry_interval_secs = 1

[notifications.smtp]
host = "{host}"
port = {port}
encryption = "none"
from = "fcp@example.com"

[logging]
level = "info"
format = "compact"
"#,
        db = stack.db_url(),
        host = mail.host(),
        port = mail.port(),
    );
    let events = Service::start(
        "events-server",
        fcp_testkit::bins::path("fcp-events-server", "fcp-events-server"),
        &events_config,
        &loopback(events_port, "/health"),
    )
    .await;

    let phone = stack.phone(&alice).await;
    let call = phone.call("933").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    call.hangup().await.unwrap();

    // The notice arrives, and says it was only a test.
    let mail = {
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        loop {
            let seen = mail.received();
            if !seen.is_empty() {
                break seen;
            }
            if std::time::Instant::now() >= deadline {
                let logs = events.kill().await;
                stack
                    .fail(&format!("no notification arrived\nevents server:\n{logs}"))
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    assert_eq!(mail[0].to, vec!["frontdesk@example.com".to_string()]);
    assert!(
        mail[0].subject().starts_with("[TEST]"),
        "{}",
        mail[0].subject()
    );
    let body = mail[0].body();
    assert!(body.contains("933"), "{body}");
    assert!(body.contains("alice"), "{body}");
    assert!(body.contains(ALICE_ELIN), "{body}");

    // And the attempt is recorded against the call, which now says
    // somebody was told.
    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    let record = &records[0];
    let mut notifications = Vec::new();
    for _ in 0..40 {
        notifications = store
            .list_emergency_notifications(&stack.seed.tenant_id, &record.call_id)
            .await
            .unwrap();
        if !notifications.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].contact.target, "frontdesk@example.com");
    assert_eq!(
        notifications[0].status,
        fcp_core::e911::notify::NotificationStatus::Delivered
    );
    let told = store
        .get_emergency_call_by_call_id(&stack.seed.tenant_id, &record.call_id)
        .await
        .unwrap()
        .expect("the record");
    assert!(told.notified, "the record says somebody was told");

    events.stop().await;
    stack.finish().await;
}

/// A tenant's own emergency-capable trunk carries its 911 call
/// (`docs/TENANT_TRUNKS.md` §5, `docs/E911_BLOCK_2.md` §12.1): the carrier
/// that tenant bought, ahead of the platform's, which until this phase was
/// the only trunk a 911 call could leave on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenants_own_emergency_trunk_carries_its_911_call() {
    let platform = Carrier::new("psap-platform", 1).await;
    let tenants = Carrier::new("psap-tenants", 1).await;
    let mut stack = stack_or_skip!(&[&platform], &[]);
    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;

    // The tenant's own emergency carrier, created through the store as the
    // API creates one.
    stack
        .seed
        .trunk_with(acme, "tenants-psap", tenants.port(), |trunk| {
            trunk.emergency_capable = true;
            // Worse priority than the platform's, to prove it is whose
            // trunk it is that decides.
            trunk.priority = 50;
        })
        .await;
    tokio::time::sleep(Duration::from_secs(4)).await;

    // The node lists both, and says whose each one is.
    let listed = stack.get("/emergency/trunks").await;
    let names: Vec<&str> = listed["trunks"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t["name"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        names.contains(&"tenants-psap") && names.contains(&"psap-platform"),
        "both are listed: {listed}"
    );
    let theirs = listed["trunks"]
        .as_array()
        .and_then(|a| a.iter().find(|t| t["name"] == "tenants-psap"))
        .expect("the tenant's trunk");
    assert_eq!(theirs["tenant_id"], acme.to_string(), "{theirs}");

    // Alice dials 911: it leaves on her tenant's carrier, not the
    // platform's.
    let alice_phone = stack.phone(&alice).await;
    let call = alice_phone.call("911").await.unwrap();
    let Some(incoming) = tenants.phone.incoming(SOON).await else {
        let why = format!(
            "the tenant's emergency carrier was not called; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    assert!(
        incoming.from.contains(ALICE_ELIN),
        "as her site's callback number: {}",
        incoming.from
    );
    assert!(
        platform
            .phone
            .incoming(Duration::from_secs(2))
            .await
            .is_none(),
        "the platform's carrier was not called"
    );

    let answered = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(answered.ended(SOON).await);
    stack.finish().await;
}

/// A TTS service that speaks every text as a second of [`NOTICE_HZ`].
async fn tone_tts() -> String {
    use axum::routing::post;
    let mut wav = Vec::new();
    let mut phase = 0.0f32;
    let samples: Vec<i16> = (0..50)
        .flat_map(|_| fcp_testkit::media::tone_frame(NOTICE_HZ as f32, &mut phase))
        .collect();
    let data_len = (samples.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    let port = free_port();
    let app = axum::Router::new().route(
        "/v1/synthesize",
        post(move || {
            let wav = wav.clone();
            async move { wav }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://127.0.0.1:{port}")
}

/// What a notification call speaks, as the stand-in TTS says it.
const NOTICE_HZ: u32 = 1100;

/// The call channel end to end (`docs/E911_BLOCK_2.md` §8.1): a test call
/// makes the events server ask this call manager to ring the tenant's
/// contact, an extension; the contact's phone rings, is answered and hears
/// the notice, and the attempt is recorded as delivered. Asked for a name
/// the tenant has no user by, the node says it could not place the call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_notification_call_rings_the_contact_and_speaks_the_notice() {
    use fcp_core::e911::notify::{ContactKind, NotifyContact};
    use fcp_core::e911::{EmergencyConfig, EmergencyStore};

    let carrier = Carrier::new("carrier-a", 10).await;
    let events_port = free_port();
    let ingest = loopback(events_port, "/ingest");
    let tts = tone_tts().await;
    let mut stack = match Stack::start_speaking(&[&carrier], &[], Some(&ingest), Some(&tts)).await {
        Some(s) => s,
        None => {
            eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
            return;
        }
    };
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let guard = stack.seed.user("guard", "guard-pw", "1050").await;
    stack.seed.site_for("alice", ALICE_ELIN).await;
    let store = fcp_storage_postgres::emergency::PostgresEmergencyStore::new(
        stack.seed.storage().pool().clone(),
    );
    let mut config = EmergencyConfig::defaults(stack.seed.tenant_id);
    config.contacts = vec![NotifyContact::new(ContactKind::Call, "1050")];
    store.put_emergency_config(&config).await.unwrap();

    // Asked directly, for someone who is not the tenant's.
    let client = reqwest::Client::new();
    let refused = client
        .post(loopback(stack.http, "/emergency/notify-call"))
        .json(&serde_json::json!({
            "to": "no-such-user", "text": "x", "tenant_id": stack.seed.tenant_id,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 503);
    let why = refused.text().await.unwrap();
    assert!(why.contains("not one of the tenant's"), "{why}");

    let events_config = format!(
        r#"
[service]
node_id = "e2e-events"

[websocket]
bind_address = "127.0.0.1:{events_port}"

[event_bus]
bus_type = "memory"

[webhook]
enabled = false
database_url = "{db}"

[notifications]
enabled = true
max_attempts = 1
retry_interval_secs = 1

[notifications.call]
call_manager_url = "http://127.0.0.1:{http}"
ring_timeout_secs = 15

[logging]
level = "info"
format = "compact"
"#,
        db = stack.db_url(),
        http = stack.http,
    );
    let events = Service::start(
        "events-server",
        fcp_testkit::bins::path("fcp-events-server", "fcp-events-server"),
        &events_config,
        &loopback(events_port, "/health"),
    )
    .await;

    let guard_phone = stack.phone(&guard).await;
    let phone = stack.phone(&alice).await;
    let call = phone.call("933").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the trunk never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    call.hangup().await.unwrap();

    // The guard's phone rings with the notice.
    let Some(ringing) = guard_phone.incoming(Duration::from_secs(25)).await else {
        let logs = events.kill().await;
        stack
            .fail(&format!(
                "the contact's phone never rang\nevents server:\n{logs}"
            ))
            .await;
    };
    let told = ringing.answer().await.unwrap();
    assert!(
        told.media().heard(NOTICE_HZ, SOON).await,
        "the contact heard nothing"
    );
    assert!(
        told.ended(Duration::from_secs(20)).await,
        "the node hung up"
    );

    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    let mut notifications = Vec::new();
    for _ in 0..80 {
        notifications = store
            .list_emergency_notifications(&stack.seed.tenant_id, &records[0].call_id)
            .await
            .unwrap();
        if notifications
            .iter()
            .any(|n| n.status == fcp_core::e911::notify::NotificationStatus::Delivered)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].contact.target, "1050");
    assert_eq!(
        notifications[0].status,
        fcp_core::e911::notify::NotificationStatus::Delivered,
        "{notifications:?}"
    );

    events.stop().await;
    stack.finish().await;
}
