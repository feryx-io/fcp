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
    phone: Phone,
}

impl Carrier {
    async fn new(name: &'static str, priority: u32) -> Self {
        // The phone never sends to its "server"; it only answers.
        let nowhere = SocketAddr::from(([127, 0, 0, 1], 9));
        let phone = Phone::new(name, "x", REALM, nowhere)
            .await
            .expect("carrier phone binds");
        Self {
            name,
            priority,
            phone,
        }
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
            port = self.phone.local_addr.port(),
        )
    }
}

impl Stack {
    /// `None` without a database. `trusted_edges` goes into `[e911]`.
    async fn start(carriers: &[&Carrier], trusted_edges: &[&str]) -> Option<Self> {
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

[logging]
level = "info"
format = "compact"

[e911]
trusted_edges = [{edges}]
{trunks}
"#,
            db = db.url(),
            rtp_end = rtp_start + 60,
            dir = dir.path().display(),
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
    let carrier = Carrier::new("psap-primary", 1).await;
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
