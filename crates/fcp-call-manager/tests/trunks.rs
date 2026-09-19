//! Tenant trunks over real SIP (`docs/TENANT_TRUNKS.md` §12, phase 2): the
//! trunks `/v1/trunks` manages carry calls. One node, two tenants, two
//! carriers — each tenant's call leaves on its own tenant's trunk, a
//! tenant with none of its own falls back to the node's `[[trunks]]`
//! unless it has said not to, and the call's record names which trunk
//! carried it. Skipped without `FCP_TEST_DATABASE_URL`.

use fcp_core::TenantId;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::seed::SeededUser;
use fcp_testkit::smoke::loopback;
use fcp_testkit::{free_port, Phone, Seed, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-call-manager");
const REALM: &str = "e2e.local";
const SOON: Duration = Duration::from_secs(10);
/// Long enough for the node's backstop read to have happened twice.
const REFRESHED: Duration = Duration::from_secs(4);
/// A carrier that is not meant to ring is given this long to prove it.
const NEVER: Duration = Duration::from_secs(2);
/// The registration loop ticks every five seconds, and the trunk it is
/// registering has to reach the node first.
const REGISTERED: Duration = Duration::from_secs(20);

/// A fresh P-256 key, PEM-encoded, for the node to sign test calls with.
fn signing_key_pem() -> String {
    use p256::pkcs8::EncodePrivateKey;
    let key = p256::SecretKey::random(&mut rand::thread_rng());
    key.to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
        .expect("PEM")
        .to_string()
}

/// A carrier: a scripted phone that answers whatever the node sends it.
struct Carrier {
    name: &'static str,
    phone: Phone,
}

impl Carrier {
    async fn new(name: &'static str) -> Self {
        // The phone never sends to its "server"; it only answers.
        let nowhere = SocketAddr::from(([127, 0, 0, 1], 9));
        let phone = Phone::new(name, "x", REALM, nowhere)
            .await
            .expect("carrier phone binds");
        Self { name, phone }
    }

    fn port(&self) -> u16 {
        self.phone.local_addr.port()
    }

    /// This carrier as one of the node's own `[[trunks]]` — the
    /// platform's, shared by every tenant that has none of its own. A
    /// carrier whose name says `psap` takes emergency calls.
    fn platform_trunk_toml(&self) -> String {
        format!(
            r#"
[[trunks]]
name = "{name}"
enabled = true
direction = "Upstream"
hosts = ["127.0.0.1"]
weight = 100
priority = 1
register = false
max_concurrent_calls = 0
emergency_capable = {emergency}

[trunks.transport]
protocol = "Udp"
port = {port}
verify_tls = false

[trunks.failover]
max_failures = 5
recovery_interval_secs = 30
health_check_enabled = false
health_check_interval_secs = 60
"#,
            name = self.name,
            port = self.port(),
            emergency = self.name.contains("psap"),
        )
    }
}

/// A running call manager, and the database its trunks come from.
struct Stack {
    service: Option<Service>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    /// Where this node answers `GET /trunks` and `/metrics`.
    http: u16,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database. `platform` are the node's own
    /// `[[trunks]]`; a test that gives none is a node whose every trunk
    /// comes from the database.
    async fn start(platform: &[&Carrier]) -> Option<Self> {
        Self::start_as("e2e-trunks", None, platform).await
    }

    /// As [`Stack::start`], with a node id of its own and, when a cluster
    /// is wanted, the Redis every node of it shares.
    async fn start_as(node_id: &str, redis: Option<&str>, platform: &[&Carrier]) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        Self::node_on(db, seed, node_id, redis, platform).await
    }

    async fn node_on(
        db: TestDatabase,
        seed: Seed,
        node_id: &str,
        redis: Option<&str>,
        platform: &[&Carrier],
    ) -> Option<Self> {
        let (service, sip, http, dir) = spawn_node(db.url(), node_id, redis, platform).await;
        Some(Self {
            service: Some(service),
            db: Some(db),
            seed,
            sip,
            http,
            _dir: dir,
        })
    }

    /// A phone that does not register: a tenant's call placed by an edge
    /// in front of this node, which names the caller in its headers rather
    /// than registering as them. A registrar authenticates against one
    /// tenant's users, and this node's realm is the first tenant's.
    async fn edge_phone(&self, user: &SeededUser) -> Phone {
        Phone::new(&user.username, &user.password, REALM, self.sip)
            .await
            .expect("phone binds")
    }

    async fn phone(&mut self, user: &SeededUser) -> Phone {
        let phone = Phone::new(&user.username, &user.password, REALM, self.sip)
            .await
            .expect("phone binds");
        match phone.register().await {
            Ok(200) => phone,
            other => {
                let why = format!("{} could not register: {other:?}", user.username);
                self.fail(&why).await
            }
        }
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

/// One call manager: its configuration, its ports and the process. A test
/// that wants a cluster starts more than one on the same database.
async fn spawn_node(
    db_url: &str,
    node_id: &str,
    redis: Option<&str>,
    platform: &[&Carrier],
) -> (Service, SocketAddr, u16, tempfile::TempDir) {
    let sip_port = free_port();
    let http = free_port();
    let rtp_start = allocate_port_pair().rtp_port;
    let dir = tempfile::tempdir().expect("tempdir");
    let trunks: String = platform.iter().map(|c| c.platform_trunk_toml()).collect();
    // A signing key of this node's own, so a trunk that asks for
    // STIR/SHAKEN has something to sign with (`docs/TENANT_TRUNKS.md`
    // §6). Made on the spot: it signs test calls and nothing else.
    let key_path = dir.path().join("stir-signing-key.pem");
    std::fs::write(&key_path, signing_key_pem()).expect("write the signing key");
    let config = format!(
        r#"
[service]
node_id = "{node_id}"

[sip]
udp_bind = "127.0.0.1:{sip_port}"
tcp_bind = "127.0.0.1:{sip_port}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"
{redis}

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

# A trunk created while the node runs reaches it on the event the API
# publishes; in this test nothing publishes one, so the backstop is what
# the node has, and it is turned right down.
[trunk_routing]
refresh_secs = 1

# Both phones place their calls from the loopback, and one of them names
# the tenant it is calling for, as a gateway or an SBC does.
[e911]
trusted_edges = ["127.0.0.0/8"]

# The rules a trunk may name (`docs/TENANT_TRUNKS.md` §6). They are the
# node's; which of them run on a call is the trunk's to say.
[stir_shaken]
enabled = true
signing_key_path = "{key_path}"
signing_cert_url = "https://certs.example/cert.pem"

[header_manipulation]
enabled = true

[[header_manipulation.rules]]
name = "carrier-account"
priority = 10
direction = "Egress"
message_type = "Request"
methods = ["INVITE"]

[[header_manipulation.rules.actions]]
action_type = "Add"
header_name = "P-Charge-Info"
value = "<sip:acct-42@carrier.example>"

[[header_manipulation.rules.actions]]
action_type = "Remove"
header_name = "User-Agent"

[[header_manipulation.rules]]
name = "never-asked-for"
priority = 20
direction = "Egress"
message_type = "Request"

[[header_manipulation.rules.actions]]
action_type = "Add"
header_name = "X-Should-Not-Appear"
value = "no"

# What a carrier's calls need on the way in: its national numbers made
# E.164 (`docs/TENANT_TRUNKS.md` §7).
[[header_manipulation.rules]]
name = "carrier-national-to-e164"
priority = 30
direction = "Ingress"
message_type = "Request"
methods = ["INVITE"]

[[header_manipulation.rules.actions]]
action_type = "Modify"
header_name = "From"
pattern = "sip:0([0-9]+)@"
value = "sip:+1$1@"
{trunks}
"#,
        db = db_url,
        node_id = node_id,
        // A node of a cluster says where the others are and beats fast,
        // so a test does not wait a quarter-minute for a takeover.
        redis = redis
            .map(|url| {
                format!(
                    "redis_url = \"{url}\"\n\n[cluster]\nenabled = true\n\
                     heartbeat_interval_secs = 1\nheartbeat_ttl_secs = 3"
                )
            })
            .unwrap_or_default(),
        rtp_end = rtp_start + 60,
        dir = dir.path().display(),
        key_path = key_path.display(),
    );
    let service = Service::start("call-manager", BIN, &config, &loopback(http, "/health")).await;
    (
        service,
        SocketAddr::from(([127, 0, 0, 1], sip_port)),
        http,
        dir,
    )
}

macro_rules! stack_or_skip {
    ($platform:expr) => {
        match Stack::start($platform).await {
            Some(s) => s,
            None => {
                eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// The headers a gateway sends naming the tenant user it is placing a
/// call for (`docs/E911_BLOCK_2.md` §6.2). With more than one tenant in
/// the database, a call to a bare address derives no tenant from its SIP
/// addressing, so this is how a test says whose call it is — the same way
/// a WebRTC gateway or an SBC in front of this node does.
struct Caller {
    tenant: String,
    user: String,
}

impl Caller {
    fn new(tenant: TenantId, user: &str) -> Self {
        Self {
            tenant: tenant.to_string(),
            user: user.to_string(),
        }
    }

    fn headers(&self) -> Vec<(&str, &str)> {
        vec![
            (fcp_core::CALLER_TENANT_HEADER, self.tenant.as_str()),
            (fcp_core::CALLER_USER_HEADER, self.user.as_str()),
        ]
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_tenants_call_leaves_on_its_own_carrier() {
    let acme_carrier = Carrier::new("acme-carrier").await;
    let globex_carrier = Carrier::new("globex-carrier").await;
    // A node with no trunks of its own: everything it routes by comes
    // from the database.
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let globex = stack.seed.tenant("globex", "globex.e2e.local").await;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    let bob = stack.seed.user_in(globex, "bob", "bob-pw", "2001").await;

    // The trunks are created while the node is already running.
    let acme_trunk = stack
        .seed
        .trunk(acme, "acme-carrier", acme_carrier.port())
        .await;
    let globex_trunk = stack
        .seed
        .trunk(globex, "globex-carrier", globex_carrier.port())
        .await;
    tokio::time::sleep(REFRESHED).await;

    // Acme's call leaves on Acme's carrier.
    let alice_phone = stack.phone(&alice).await;
    let alice_caller = Caller::new(acme, "alice");
    let alice_call = alice_phone
        .call_with_headers("+15125550100", &alice_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = acme_carrier.phone.incoming(SOON).await else {
        let why = format!(
            "Acme's carrier never rang; the call is {:?}",
            alice_call.phase()
        );
        stack.fail(&why).await;
    };
    assert!(
        incoming
            .request_uri()
            .contains(&format!("127.0.0.1:{}", acme_carrier.port())),
        "{}",
        incoming.request_uri()
    );
    let acme_leg = incoming.answer().await.unwrap();
    alice_call.answered(SOON).await.unwrap();

    // And Globex's on Globex's — the same node, the same dialled number.
    let bob_phone = stack.edge_phone(&bob).await;
    let bob_caller = Caller::new(globex, "bob");
    let bob_call = bob_phone
        .call_with_headers("+15125550100", &bob_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = globex_carrier.phone.incoming(SOON).await else {
        let why = format!(
            "Globex's carrier never rang; the call is {:?}",
            bob_call.phase()
        );
        stack.fail(&why).await;
    };
    assert!(
        incoming
            .request_uri()
            .contains(&format!("127.0.0.1:{}", globex_carrier.port())),
        "{}",
        incoming.request_uri()
    );
    let globex_leg = incoming.answer().await.unwrap();
    bob_call.answered(SOON).await.unwrap();

    // Neither tenant's call reached the other's carrier.
    assert!(
        acme_carrier.phone.incoming(NEVER).await.is_none(),
        "Acme's carrier took a second call; only Alice's was its own"
    );

    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(alice_call.hangup().await.unwrap(), 200);
    assert_eq!(bob_call.hangup().await.unwrap(), 200);
    assert!(acme_leg.ended(SOON).await, "Acme's leg ends");
    assert!(globex_leg.ended(SOON).await, "Globex's leg ends");

    // Each record names the trunk that carried it, by id: both trunks
    // could as easily have been called "carrier".
    let records = stack.seed.cdrs(None, 2, SOON).await;
    let trunk_of = |tenant: TenantId| {
        records
            .iter()
            .find(|r| r.tenant_id == tenant)
            .unwrap_or_else(|| panic!("a record for {tenant}: {records:#?}"))
            .trunk_id
            .clone()
    };
    assert_eq!(trunk_of(acme), Some(acme_trunk), "{records:#?}");
    assert_eq!(trunk_of(globex), Some(globex_trunk), "{records:#?}");

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenant_without_trunks_falls_back_to_the_platforms_unless_it_refuses() {
    let shared = Carrier::new("shared-carrier").await;
    // This time the node has a trunk of its own: the platform's.
    let mut stack = stack_or_skip!(&[&shared]);

    let globex = stack.seed.tenant("globex", "globex.e2e.local").await;
    let bob = stack.seed.user_in(globex, "bob", "bob-pw", "2001").await;
    let bob_phone = stack.edge_phone(&bob).await;
    let bob_caller = Caller::new(globex, "bob");

    // A tenant with no trunks of its own calls out on the platform's.
    let call = bob_phone
        .call_with_headers("+15125550100", &bob_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = shared.phone.incoming(SOON).await else {
        let why = format!(
            "the platform's carrier never rang for a tenant with no trunks; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    // A tenant whose calls may not leave on a shared carrier has nowhere
    // to send this one.
    stack.seed.platform_trunk_fallback(globex, false).await;
    tokio::time::sleep(REFRESHED).await;

    let refused = bob_phone
        .call_with_headers("+15125550100", &bob_caller.headers())
        .await
        .unwrap();
    assert!(
        shared.phone.incoming(NEVER).await.is_none(),
        "a tenant that refuses the platform's trunks does not leave on one"
    );
    assert!(
        refused.failed(SOON).await.is_some(),
        "the call is refused rather than left ringing"
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_presents_its_own_identity_timer_and_dtmf() {
    let carrier = Carrier::new("billing-carrier").await;
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, "billing-carrier", carrier.port(), |trunk| {
            trunk.caller_id_override = Some("+15125550100".to_string());
            trunk.from_domain = Some("carrier.example".to_string());
            trunk.session_timer_secs = 1800;
            trunk.dtmf = fcp_core::trunk::TrunkDtmfConfig {
                mode: fcp_core::trunk::TrunkDtmfMode::Detect,
            };
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let alice_caller = Caller::new(acme, "alice");
    let call = alice_phone
        .call_with_headers("+15125559999", &alice_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };

    // The identity the trunk bills by, not the extension behind it.
    assert!(
        incoming.from.contains("+15125550100@carrier.example"),
        "From: {}",
        incoming.from
    );
    assert_eq!(
        incoming.header("P-Asserted-Identity").as_deref(),
        Some("<sip:+15125550100@carrier.example>"),
        "the carrier is told which identity the trunk asserts"
    );

    // The session refresh it asks for (RFC 4028).
    assert_eq!(
        incoming.header("Session-Expires").as_deref(),
        Some("1800;refresher=uac")
    );
    assert_eq!(incoming.header("Min-SE").as_deref(), Some("90"));
    assert!(
        incoming
            .headers_named("Supported")
            .iter()
            .any(|v| v.contains("timer")),
        "Supported: {:?}",
        incoming.headers_named("Supported")
    );

    // And no RFC 2833 offered to a trunk that takes DTMF as SIP INFO.
    let offer = String::from_utf8_lossy(incoming.body()).to_string();
    assert!(
        !offer.contains("telephone-event"),
        "the offer keeps RFC 2833 the trunk does not take:\n{offer}"
    );

    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_that_wants_srtp_is_offered_it() {
    let carrier = Carrier::new("srtp-carrier").await;
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, "srtp-carrier", carrier.port(), |trunk| {
            trunk.srtp = fcp_core::trunk::TrunkSrtpConfig {
                mode: fcp_core::trunk::SrtpMode::Preferred,
                allowed_suites: Vec::new(),
            };
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let alice_caller = Caller::new(acme, "alice");
    let call = alice_phone
        .call_with_headers("+15125559999", &alice_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };

    let offer = String::from_utf8_lossy(incoming.body()).to_string();
    assert!(
        offer.contains("RTP/SAVP"),
        "a trunk that wants SRTP is offered an encrypted profile:\n{offer}"
    );
    assert!(
        offer.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:"),
        "and the key this node will send with:\n{offer}"
    );

    // This trunk only prefers it, so an answer in the clear still connects.
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_that_requires_srtp_refuses_an_answer_without_it() {
    let carrier = Carrier::new("srtp-only-carrier").await;
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, "srtp-only-carrier", carrier.port(), |trunk| {
            trunk.srtp = fcp_core::trunk::TrunkSrtpConfig {
                mode: fcp_core::trunk::SrtpMode::Required,
                allowed_suites: Vec::new(),
            };
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let alice_caller = Caller::new(acme, "alice");
    let call = alice_phone
        .call_with_headers("+15125559999", &alice_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };

    // The scripted carrier answers in the clear, which this trunk does not
    // allow: the leg is hung up and the caller refused rather than sent
    // unencrypted media the carrier expected encrypted.
    let leg = incoming.answer().await.unwrap();
    let code = call.failed(SOON).await;
    if code.is_none() {
        let why = format!(
            "a call to a trunk requiring SRTP was not refused; it is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    }
    assert_eq!(code, Some(488), "Not Acceptable Here");
    assert!(
        leg.ended(SOON).await,
        "and the carrier's leg is hung up, not left up"
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_runs_the_header_rules_it_names_and_no_others() {
    let carrier = Carrier::new("hmr-carrier").await;
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, "hmr-carrier", carrier.port(), |trunk| {
            trunk.header_rules = vec!["carrier-account".to_string()];
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let alice_caller = Caller::new(acme, "alice");
    let call = alice_phone
        .call_with_headers("+15125559999", &alice_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };

    // The rule this trunk names ran, on the finished INVITE: it added a
    // header of its own and took away one the builder had written.
    assert_eq!(
        incoming.header("P-Charge-Info").as_deref(),
        Some("<sip:acct-42@carrier.example>")
    );
    assert!(
        incoming.header("User-Agent").is_none(),
        "a rule can remove a header this node itself wrote"
    );
    // And the rule it did not name did not run.
    assert!(
        incoming.header("X-Should-Not-Appear").is_none(),
        "only the rules the trunk names run on its calls"
    );
    // The call is otherwise ordinary.
    assert!(
        incoming.header("Call-ID").is_some() && incoming.header("Via").is_some(),
        "the request is still a SIP request"
    );

    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_that_asks_for_stir_shaken_gets_a_signed_call() {
    let carrier = Carrier::new("signed-carrier").await;
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, "signed-carrier", carrier.port(), |trunk| {
            trunk.caller_id_override = Some("+15125550100".to_string());
            trunk.stir_shaken = fcp_core::trunk::TrunkStirShakenConfig {
                sign_mode: fcp_core::trunk::StirSignMode::Enabled,
                attestation: fcp_core::trunk::AttestationLevel::A,
                ..Default::default()
            };
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let alice_caller = Caller::new(acme, "alice");
    let call = alice_phone
        .call_with_headers("+15125559999", &alice_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };

    let Some(identity) = incoming.header("Identity") else {
        let why = format!(
            "a trunk that asks for STIR/SHAKEN sent no Identity header; it sent {:?}",
            incoming.headers_named("Identity")
        );
        stack.fail(&why).await;
    };
    assert!(identity.contains("ppt=shaken"), "Identity: {identity}");
    assert!(identity.contains("alg=ES256"), "Identity: {identity}");
    assert!(
        identity.contains("info=<https://certs.example/cert.pem>"),
        "the header names the certificate this node signs with: {identity}"
    );
    // The PASSporT covers the numbers the carrier sees: the trunk's own
    // caller ID, not the extension behind it.
    let claims = passport_claims(&identity);
    assert_eq!(claims["attest"], "A", "{claims}");
    assert_eq!(
        claims["orig"]["tn"], "15125550100",
        "RFC 8225 §5.2.1: orig.tn is one number, as a string: {claims}"
    );
    assert_eq!(claims["dest"]["tn"][0], "15125559999", "{claims}");

    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    stack.finish().await;
}

/// The claims of the PASSporT in an `Identity` header (RFC 8225), read
/// without verifying it: the signature is the carrier's business, the
/// claims are this test's.
/// A number `tenant` owns, as a carrier would dial it, answered by a flow
/// of that tenant's that answers and waits: which tenant's call it became
/// is then on its record.
async fn seed_answered_number(stack: &Stack, tenant: TenantId, e164: &str) {
    use fcp_core::storage::{LineBindingStore, PhoneNumberStore};
    let flow_id = format!("answer-{e164}");
    let flow: fcp_flow::FlowDefinition = serde_json::from_value(serde_json::json!({
        "id": flow_id,
        "name": "Answer",
        "version": 1,
        "initial_node": "answer",
        "enabled": true,
        "tenant_id": tenant.to_string(),
        "nodes": {
            "answer": {"id": "answer", "name": "Answer", "type": "answer", "next": "wait"},
            "wait": {"id": "wait", "name": "Wait", "type": "pause", "duration": 30, "next": "done"},
            "done": {"id": "done", "name": "Hang up", "type": "hangup"}
        }
    }))
    .expect("flow fixture deserializes");
    fcp_flow::FlowStore::insert_flow(stack.seed.storage(), flow)
        .await
        .expect("seed flow");
    let number = fcp_core::PhoneNumber::new(e164.to_string(), tenant).expect("valid number");
    let mut binding =
        fcp_core::LineBinding::new(number.id, None, fcp_core::BindingType::Ivr, tenant);
    binding.flow_id = Some(flow_id);
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_carriers_call_is_the_tenant_its_trunk_says() {
    // One carrier, from one address, serving two tenants — the case a
    // carrier's address alone cannot settle, so the number does.
    let carrier = Carrier::new("shared-address").await;
    let mut stack = stack_or_skip!(&[]);

    let acme = stack.seed.tenant_id;
    let globex = stack.seed.tenant("globex", "globex.e2e.local").await;
    let both_ways = |trunk: &mut fcp_core::Trunk| {
        trunk.direction = fcp_core::TrunkDirection::Bidirectional;
    };
    let acme_trunk = stack
        .seed
        .trunk_with(acme, "acme-in", carrier.port(), both_ways)
        .await;
    let globex_trunk = stack
        .seed
        .trunk_with(globex, "globex-in", carrier.port(), both_ways)
        .await;

    // Each tenant's number is answered by a flow of that tenant's.
    seed_answered_number(&stack, acme, "+15125550111").await;
    seed_answered_number(&stack, globex, "+15125550222").await;
    tokio::time::sleep(REFRESHED).await;

    // The carrier's calls are addressed to this node, as a carrier's are:
    // nothing in them says which tenant they are for.
    let from_carrier = Phone::new("carrier-caller", "x", "carrier.example", stack.sip)
        .await
        .expect("carrier phone binds");
    for number in ["+15125550111", "+15125550222"] {
        let call = from_carrier.call(number).await.unwrap();
        if call.answered(SOON).await.is_err() {
            let why = format!("{number} was not answered; the call is {:?}", call.phase());
            stack.fail(&why).await;
        }
        assert_eq!(call.hangup().await.unwrap(), 200);
    }

    // A number neither owns, from an address both take calls from, is
    // nobody's call.
    let call = from_carrier.call("+15125550333").await.unwrap();
    assert_eq!(call.failed(SOON).await, Some(404));

    let records = stack.seed.cdrs(None, 2, SOON).await;
    let record_for = |tenant: TenantId| {
        records
            .iter()
            .find(|r| r.tenant_id == tenant)
            .unwrap_or_else(|| panic!("a record for {tenant}: {records:#?}"))
    };
    assert_eq!(record_for(acme).trunk_id, Some(acme_trunk), "{records:#?}");
    assert_eq!(
        record_for(globex).trunk_id,
        Some(globex_trunk),
        "{records:#?}"
    );

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_carriers_call_runs_the_ingress_rules_its_trunk_names() {
    let mut stack = stack_or_skip!(&[]);
    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    // The carrier sends from the loopback; the trunk takes its calls and
    // names the rule that makes its numbers E.164.
    stack
        .seed
        .trunk_with(acme, "national-carrier", 9, |trunk| {
            trunk.direction = fcp_core::TrunkDirection::Bidirectional;
            trunk.header_rules = vec!["carrier-national-to-e164".to_string()];
        })
        .await;
    tokio::time::sleep(REFRESHED).await;
    let alice_phone = stack.phone(&alice).await;

    let from_carrier = Phone::new("05125550999", "x", "carrier.example", stack.sip)
        .await
        .expect("carrier phone binds");
    let call = from_carrier.call("1001").await.unwrap();
    let Some(incoming) = alice_phone.incoming(SOON).await else {
        let why = format!("Alice never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    assert!(
        incoming.from.contains("+15125550999"),
        "the caller as the trunk's rule leaves it: {}",
        incoming.from
    );
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunks_channels_are_shared_by_its_calls_in_and_out() {
    let carrier = Carrier::new("one-channel").await;
    let mut stack = stack_or_skip!(&[]);
    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, "one-channel", carrier.port(), |trunk| {
            trunk.direction = fcp_core::TrunkDirection::Bidirectional;
            trunk.max_concurrent_calls = 1;
        })
        .await;
    tokio::time::sleep(REFRESHED).await;
    let alice_phone = stack.phone(&alice).await;
    let from_carrier = Phone::new("carrier-caller", "x", "carrier.example", stack.sip)
        .await
        .expect("carrier phone binds");

    // Alice's call out takes the trunk's one channel...
    let out = alice_phone
        .call_with_headers("+15125559999", &Caller::new(acme, "alice").headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", out.phase());
        stack.fail(&why).await;
    };
    let out_leg = incoming.answer().await.unwrap();
    out.answered(SOON).await.unwrap();

    // ...so the carrier's call in finds none.
    let refused = from_carrier.call("1001").await.unwrap();
    assert_eq!(refused.failed(SOON).await, Some(503));

    // When it ends, the channel is free again, in either direction.
    assert_eq!(out.hangup().await.unwrap(), 200);
    assert!(out_leg.ended(SOON).await);
    let call = from_carrier.call("1001").await.unwrap();
    let Some(incoming) = alice_phone.incoming(SOON).await else {
        let why = format!(
            "the freed channel was not reused; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();

    // And while that call holds it, a second one in is refused too.
    let second = from_carrier.call("1001").await.unwrap();
    assert_eq!(second.failed(SOON).await, Some(503));

    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);
    stack.finish().await;
}

/// A call from a tenant user out on a trunk that takes DTMF as `mode`,
/// answered at both ends: the caller's phone, its call and the carrier's
/// leg.
async fn answered_dtmf_call(
    stack: &mut Stack,
    carrier: &Carrier,
    mode: fcp_core::trunk::TrunkDtmfMode,
) -> (Phone, fcp_testkit::Call, fcp_testkit::Call) {
    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    stack
        .seed
        .trunk_with(acme, carrier.name, carrier.port(), |trunk| {
            trunk.dtmf = fcp_core::trunk::TrunkDtmfConfig { mode };
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let call = alice_phone
        .call_with_headers("+15125559999", &Caller::new(acme, "alice").headers())
        .await
        .unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        let why = format!("the carrier never rang; the call is {:?}", call.phase());
        stack.fail(&why).await;
    };
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    // Media flowing both ways before any digit is sent.
    tokio::time::sleep(Duration::from_millis(600)).await;
    (alice_phone, call, leg)
}

/// Whether the carrier is sent a SIP INFO carrying `digit`.
async fn carrier_got_info(carrier: &Carrier, digit: char, within: Duration) -> bool {
    let signal = format!("Signal={digit}");
    carrier
        .phone
        .expect(
            within,
            |ev| matches!(ev, fcp_testkit::PhoneEvent::Info { body, .. } if body.contains(&signal)),
        )
        .await
        .is_some()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_that_takes_sip_info_is_sent_the_callers_digits_as_info() {
    let carrier = Carrier::new("info-carrier").await;
    let mut stack = stack_or_skip!(&[]);
    let (_phone, call, leg) =
        answered_dtmf_call(&mut stack, &carrier, fcp_core::trunk::TrunkDtmfMode::Detect).await;

    // The caller presses 5 the way a phone does, as telephone-event.
    call.media().send_dtmf('5');
    if !carrier_got_info(&carrier, '5', SOON).await {
        stack
            .fail("the carrier never received the caller's digit as SIP INFO")
            .await;
    }
    assert_eq!(
        leg.media().heard_dtmf(NEVER).await,
        None,
        "telephone-event went to a trunk that did not negotiate it"
    );

    // And the trunk's own INFO reaches a caller that takes telephone-event
    // as telephone-event.
    assert_eq!(leg.send_info_dtmf('9').await.unwrap(), 200);
    assert_eq!(call.media().heard_dtmf(SOON).await, Some('9'));

    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);
    stack.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_trunk_that_takes_rfc2833_is_played_the_callers_info_digits() {
    let carrier = Carrier::new("rfc2833-carrier").await;
    let mut stack = stack_or_skip!(&[]);
    let (_phone, call, leg) =
        answered_dtmf_call(&mut stack, &carrier, fcp_core::trunk::TrunkDtmfMode::Relay).await;

    // A caller that sends its digit as SIP INFO; the trunk hears it as
    // telephone-event, and is not sent the INFO.
    assert_eq!(call.send_info_dtmf('7').await.unwrap(), 200);
    assert_eq!(leg.media().heard_dtmf(SOON).await, Some('7'));
    assert!(
        !carrier_got_info(&carrier, '7', NEVER).await,
        "the trunk was sent INFO it does not take"
    );

    // Telephone-event itself still goes straight through.
    call.media().send_dtmf('3');
    assert_eq!(leg.media().heard_dtmf(SOON).await, Some('3'));

    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);
    stack.finish().await;
}

fn passport_claims(identity: &str) -> serde_json::Value {
    use base64::Engine;
    let token = identity.split(';').next().unwrap_or(identity).trim();
    let payload = token.split('.').nth(1).expect("a JWS has three parts");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("the payload is base64url");
    serde_json::from_slice(&bytes).expect("the payload is JSON")
}

/// A registering trunk binds its own account at the carrier
/// (`docs/TENANT_TRUNKS.md` §8): `From` and `To` name the carrier's
/// account and the challenge is answered with that account's own
/// credentials, not this node's identity. `Contact` is this node, which is
/// where the carrier's calls for the account arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_registering_trunk_binds_its_own_account_at_the_carrier() {
    let carrier = Carrier::new("registrar-carrier").await;
    carrier.phone.challenge_registrations("carrier.example");
    let mut stack = stack_or_skip!(&[]);
    let acme = stack.seed.tenant_id;

    stack
        .seed
        .trunk_with(acme, "registering", carrier.port(), |trunk| {
            trunk.register = true;
            trunk.register_expiry = Some(120);
            trunk.credentials = Some(fcp_core::trunk::TrunkCredentials {
                username: "acct-4471".to_string(),
                password: "s3cret".to_string(),
                realm: Some("carrier.example".to_string()),
            });
        })
        .await;

    let bound = carrier
        .phone
        .registrations_settle(REGISTERED, |seen| !seen.is_empty())
        .await;
    if !bound {
        stack.fail("the trunk never registered").await;
    }
    let seen = carrier.phone.registrations().remove(0);
    assert!(
        seen.account.contains("sip:acct-4471@carrier.example"),
        "the account, not this node: {}",
        seen.account
    );
    assert!(
        seen.from.contains("sip:acct-4471@carrier.example"),
        "and it is who the REGISTER is from: {}",
        seen.from
    );
    assert_eq!(
        seen.auth_username.as_deref(),
        Some("acct-4471"),
        "the trunk's own credentials answered the challenge"
    );
    assert!(
        seen.contact.contains(&stack.sip.port().to_string()),
        "the binding points at this node: {}",
        seen.contact
    );
    assert_eq!(seen.expires, 120, "the trunk's own expiry");
    stack.finish().await;
}

/// One node of a cluster registers a registering trunk
/// (`docs/TENANT_TRUNKS.md` §8, §11): two call managers load the same
/// tenant trunk, the carrier is asked for one binding and not two, and
/// when the node holding it goes the other takes it over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_node_of_a_cluster_registers_a_trunk_and_another_takes_it_over() {
    let Some(redis) = fcp_testkit::cluster::private_redis_url() else {
        eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
        return;
    };
    let carrier = Carrier::new("cluster-carrier").await;
    let Some(mut first) = Stack::start_as("e2e-trunks-a", Some(&redis), &[]).await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let acme = first.seed.tenant_id;
    first
        .seed
        .trunk_with(acme, "shared-carrier", carrier.port(), |trunk| {
            trunk.register = true;
            trunk.register_expiry = Some(120);
            trunk.credentials = Some(fcp_core::trunk::TrunkCredentials {
                username: "acct-shared".to_string(),
                password: "s3cret".to_string(),
                realm: Some("carrier.example".to_string()),
            });
        })
        .await;

    // A second node on the same database and the same Redis: both load
    // the trunk, and both would have registered it before this phase.
    let (second, second_sip, _second_http, _second_dir) = spawn_node(
        first.db.as_ref().expect("the test database").url(),
        "e2e-trunks-b",
        Some(&redis),
        &[],
    )
    .await;

    let bound = |seen: &[fcp_testkit::phone::SeenRegistration]| seen.iter().any(|r| r.expires > 0);
    if !carrier.phone.registrations_settle(REGISTERED, bound).await {
        first.fail("neither node registered the trunk").await;
    }

    // Long enough for both nodes to have seen each other's claim: the
    // node that loads a trunk first may hold it for a tick before the
    // lower node id takes it over, and the one that loses it gives its
    // binding up rather than leave the carrier with two.
    tokio::time::sleep(Duration::from_secs(12)).await;
    let holder = live_binding(&carrier.phone);
    let Some(first_holder) = holder else {
        let b_logs = second.kill().await;
        eprintln!("=== NODE B ===\n{b_logs}");
        first
            .fail(&format!(
                "no node holds the binding: {:?}",
                carrier.phone.registrations()
            ))
            .await;
    };

    // The node holding it goes. The other has it within a beat or two of
    // the heartbeat expiring.
    let held_by_first = first_holder.contains(&first.sip.port().to_string());
    let mut survivor_node = None;
    if held_by_first {
        if let Some(service) = first.service.take() {
            service.kill().await;
        }
        survivor_node = Some(second);
    } else {
        assert!(
            first_holder.contains(&second_sip.port().to_string()),
            "the binding is one of the two nodes': {first_holder}"
        );
        second.kill().await;
    }

    let taken_over = carrier
        .phone
        .registrations_settle(REGISTERED, |seen| {
            seen.iter()
                .any(|r| r.expires > 0 && r.contact != first_holder)
        })
        .await;
    assert!(
        taken_over,
        "the surviving node never took the binding over: {:?}",
        carrier.phone.registrations()
    );
    let survivor = carrier
        .phone
        .registrations()
        .into_iter()
        .rfind(|r| r.expires > 0)
        .expect("a binding");
    assert_ne!(survivor.contact, first_holder, "and it is the other node's");
    assert!(
        survivor.account.contains("sip:acct-shared@carrier.example"),
        "and still as the trunk's own account: {}",
        survivor.account
    );

    if let Some(node) = survivor_node {
        node.stop().await;
    }
    first.finish().await;
}

/// The one binding a carrier is holding for a trunk right now: the
/// contact of the last REGISTER that was not withdrawn. `None` when every
/// binding has been given up, and a panic when two nodes hold one at once,
/// which is what this phase is for.
fn live_binding(carrier: &Phone) -> Option<String> {
    let mut live: Vec<String> = Vec::new();
    for seen in carrier.registrations() {
        live.retain(|c| *c != seen.contact);
        if seen.expires > 0 {
            live.push(seen.contact);
        }
    }
    assert!(
        live.len() <= 1,
        "two nodes hold a binding for one trunk at once: {live:?}"
    );
    live.pop()
}

/// A node answers for the trunks it routes by (`docs/TENANT_TRUNKS.md`
/// §9): its own probes, its own calls, its own binding. The tenants'
/// trunks are in there beside the platform's, which is what this endpoint
/// adds over `/emergency/trunks`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_says_what_it_sees_of_its_trunks() {
    let carrier = Carrier::new("seen-carrier").await;
    let stack = stack_or_skip!(&[&carrier]);
    let acme = stack.seed.tenant_id;
    let tenant_trunk = stack
        .seed
        .trunk_with(acme, "tenants-own", carrier.port(), |trunk| {
            trunk.register = true;
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let (code, body) =
        fcp_testkit::smoke::http_get(&fcp_testkit::smoke::loopback(stack.http, "/trunks"))
            .await
            .expect("the node answers");
    assert_eq!(code, 200, "{body}");
    let seen: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(seen["node_id"], "e2e-trunks", "{seen}");
    assert_eq!(seen["routes_by_trunks"], true, "{seen}");

    let trunks = seen["trunks"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = trunks.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"tenants-own") && names.contains(&"seen-carrier"),
        "the tenants' trunks beside the platform's: {names:?}"
    );

    let tenants = trunks
        .iter()
        .find(|t| t["name"] == "tenants-own")
        .expect("the tenant's trunk");
    assert_eq!(tenants["trunk_id"], tenant_trunk.to_string(), "{tenants}");
    assert_eq!(tenants["tenant_id"], acme.to_string(), "{tenants}");
    assert!(
        tenants["registration"].is_string(),
        "a registering trunk says what its binding is doing: {tenants}"
    );
    let platform = trunks
        .iter()
        .find(|t| t["name"] == "seen-carrier")
        .expect("the platform's trunk");
    assert!(
        platform["tenant_id"].is_null(),
        "the platform's trunk is nobody's: {platform}"
    );
    assert!(
        platform["registration"].is_null(),
        "and it does not register: {platform}"
    );
    stack.finish().await;
}

/// The calls a trunk carried, refused for capacity and answered are
/// counted per trunk and tenant on `/metrics` (§9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_a_trunk_carried_is_counted_per_trunk_and_tenant() {
    let carrier = Carrier::new("counted-carrier").await;
    let mut stack = stack_or_skip!(&[]);
    let acme = stack.seed.tenant_id;
    let trunk_id = stack
        .seed
        .trunk_with(acme, "counted", carrier.port(), |trunk| {
            trunk.max_concurrent_calls = 1;
        })
        .await;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    tokio::time::sleep(REFRESHED).await;
    let alice_phone = stack.phone(&alice).await;

    // One call out takes the trunk's only channel; the second is refused
    // for capacity, which is the pair of outcomes §9 asks for.
    let first = alice_phone.call("+15125557001").await.unwrap();
    let Some(incoming) = carrier.phone.incoming(SOON).await else {
        stack.fail("the carrier was not called").await;
    };
    let carried = incoming.answer().await.unwrap();
    first.answered(SOON).await.unwrap();
    let second = alice_phone.call("+15125557002").await.unwrap();
    assert_eq!(second.failed(SOON).await, Some(503), "the channel is taken");

    let (code, metrics) =
        fcp_testkit::smoke::http_get(&fcp_testkit::smoke::loopback(stack.http, "/metrics"))
            .await
            .expect("the node answers");
    assert_eq!(code, 200);
    let want = format!("trunk_id=\"{trunk_id}\",tenant=\"{acme}\"");
    assert!(
        metrics.contains(&format!("{want},outcome=\"attempted\"}} 1")),
        "the call the trunk took: {metrics}"
    );
    assert!(
        metrics.contains(&format!("{want},outcome=\"rejected_capacity\"}} 1")),
        "and one was refused for capacity: {metrics}"
    );
    assert!(
        metrics.contains(&format!("{want},outcome=\"answered\"}} 1")),
        "and one was carried: {metrics}"
    );

    assert_eq!(first.hangup().await.unwrap(), 200);
    assert!(carried.ended(SOON).await);
    stack.finish().await;
}

/// A tenant's own emergency-capable trunk carries its 911 call, ahead of
/// the platform's (`docs/TENANT_TRUNKS.md` §5, `docs/E911_BLOCK_2.md`
/// §12.1): the carrier the tenant bought is the one its emergency calls
/// leave on, and the record says which trunk that was by id.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tenants_911_call_leaves_on_its_own_emergency_trunk() {
    let platform_psap = Carrier::new("platform-psap").await;
    let acme_psap = Carrier::new("acme-psap").await;
    let mut stack = stack_or_skip!(&[&platform_psap]);

    let acme = stack.seed.tenant_id;
    let alice = stack.seed.user("alice", "alice-pw", "1001").await;
    // A worse priority than the platform's trunk, so what decides is whose
    // trunk it is and not the number beside it.
    let acme_trunk = stack
        .seed
        .trunk_with(acme, "acme-psap", acme_psap.port(), |t| {
            t.emergency_capable = true;
            t.priority = 50;
        })
        .await;
    tokio::time::sleep(REFRESHED).await;

    let alice_phone = stack.phone(&alice).await;
    let call = alice_phone.call("911").await.unwrap();
    let Some(incoming) = acme_psap.phone.incoming(SOON).await else {
        let why = format!(
            "the tenant's emergency trunk never rang; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    assert!(
        platform_psap.phone.incoming(NEVER).await.is_none(),
        "the platform's emergency trunk is the last resort, not the first"
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    let records = stack
        .seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    assert_eq!(records[0].trunk_id, Some(acme_trunk), "{records:#?}");
    assert_eq!(records[0].trunk_name.as_deref(), Some("acme-psap"));

    stack.finish().await;
}

/// A tenant with no emergency trunk of its own reaches the platform's,
/// even having refused the platform's trunks for its ordinary calls: that
/// is a billing and routing preference, and nobody's 911 call fails for
/// it (`docs/TENANT_TRUNKS.md` §13 question 6).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_platforms_emergency_trunk_carries_anyones_911_call() {
    let platform_psap = Carrier::new("platform-psap").await;
    let globex_carrier = Carrier::new("globex-carrier").await;
    let mut stack = stack_or_skip!(&[&platform_psap]);

    let globex = stack.seed.tenant("globex", "globex.e2e.local").await;
    let bob = stack.seed.user_in(globex, "bob", "bob-pw", "2001").await;
    stack.seed.platform_trunk_fallback(globex, false).await;
    // Globex's own carrier, which does not take emergency calls.
    stack
        .seed
        .trunk(globex, "globex-carrier", globex_carrier.port())
        .await;
    tokio::time::sleep(REFRESHED).await;

    let bob_phone = stack.edge_phone(&bob).await;
    let bob_caller = Caller::new(globex, "bob");
    let call = bob_phone
        .call_with_headers("911", &bob_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = platform_psap.phone.incoming(SOON).await else {
        let why = format!(
            "the platform's emergency trunk never rang for a tenant that refused the \
             platform's trunks; the call is {:?}",
            call.phase()
        );
        stack.fail(&why).await;
    };
    let leg = incoming.answer().await.unwrap();
    call.answered(SOON).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(call.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    let records = stack
        .seed
        .emergency_calls_in(globex, 1, SOON, |r| r.ended_at.is_some())
        .await;
    assert_eq!(
        records[0].trunk_id,
        Some(fcp_core::TrunkId::platform("platform-psap")),
        "{records:#?}"
    );

    // Its ordinary calls still keep off the platform's trunks.
    let ordinary = bob_phone
        .call_with_headers("+15125550100", &bob_caller.headers())
        .await
        .unwrap();
    let Some(incoming) = globex_carrier.phone.incoming(SOON).await else {
        let why = format!(
            "Globex's own carrier never rang; the call is {:?}",
            ordinary.phase()
        );
        stack.fail(&why).await;
    };
    let leg = incoming.answer().await.unwrap();
    ordinary.answered(SOON).await.unwrap();
    assert!(
        platform_psap.phone.incoming(NEVER).await.is_none(),
        "an ordinary call from a tenant that refused the platform's trunks"
    );
    assert_eq!(ordinary.hangup().await.unwrap(), 200);
    assert!(leg.ended(SOON).await);

    stack.finish().await;
}
