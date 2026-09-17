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
    /// platform's, shared by every tenant that has none of its own.
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
        )
    }
}

/// A running call manager, and the database its trunks come from.
struct Stack {
    service: Option<Service>,
    db: Option<TestDatabase>,
    seed: Seed,
    sip: SocketAddr,
    _dir: tempfile::TempDir,
}

impl Stack {
    /// `None` without a database. `platform` are the node's own
    /// `[[trunks]]`; a test that gives none is a node whose every trunk
    /// comes from the database.
    async fn start(platform: &[&Carrier]) -> Option<Self> {
        let db = TestDatabase::create().await?;
        let seed = Seed::new(db.url(), REALM).await;
        let sip_port = free_port();
        let http = free_port();
        let rtp_start = allocate_port_pair().rtp_port;
        let dir = tempfile::tempdir().expect("tempdir");
        let trunks: String = platform.iter().map(|c| c.platform_trunk_toml()).collect();
        let config = format!(
            r#"
[service]
node_id = "e2e-trunks"

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

# A trunk created while the node runs reaches it on the event the API
# publishes; in this test nothing publishes one, so the backstop is what
# the node has, and it is turned right down.
[trunk_routing]
refresh_secs = 1

# Both phones place their calls from the loopback, and one of them names
# the tenant it is calling for, as a gateway or an SBC does.
[e911]
trusted_edges = ["127.0.0.0/8"]
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
