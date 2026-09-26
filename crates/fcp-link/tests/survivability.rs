//! Survivability, proved (`docs/HYBRID_CONTROL_PLANE.md` phase 6): the
//! floor cuts the link between a site and its cloud, as a network would,
//! and the site goes on alone. Phones keep registering and calling one
//! another, a local trunk keeps carrying the site's calls out, the
//! site's records and events wait; the cloud says the site is
//! unreachable meanwhile. On reconnect the cloud says it is back, what
//! the cloud changed reaches the site, what the site wrote reaches the
//! cloud, and the site's link registers again so the cloud's calls reach
//! it. And what the cloud sends a site mid-call — a re-INVITE, a BYE —
//! goes down the connection the site opened. Gated on
//! `FCP_TEST_DATABASE_URL` and `FCP_TEST_REDIS_URL`; skipped when unset.

mod common;

use common::{
    cloud_with_site_opts, eventually, stub_events_server, until, CallManagerToml, CloudOptions,
};
use fcp_core::storage::{CdrQuery, CdrStore};
use fcp_testkit::cut::CutLink;
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{free_port, http_request, loopback, toml_path};
use fcp_testkit::{bins, Phone, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

const REALM: &str = "fcp.local";
const SITE_NUMBER: &str = "+15125550111";
const OUTSIDE_NUMBER: &str = "+15125550999";
const ALICE_PASSWORD: &str = "Alice-at-the-site-2026!";
const BOB_PASSWORD: &str = "Bob-at-the-site-2026!";
const CAROL_PASSWORD: &str = "Carol-made-during-the-cut-2026!";
const SOON: Duration = Duration::from_secs(10);

/// `assert!` that shows every node's log when it fails.
macro_rules! check {
    ($nodes:expr, $cond:expr, $($arg:tt)+) => {
        if !$cond {
            $nodes.fail(&format!($($arg)+)).await;
        }
    };
}
/// Register until the registrar answers 200, or give up.
async fn registers(what: &str, phone: &Phone) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let code = phone.register().await.unwrap();
        if code == 200 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: the registrar kept answering {code}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// The SBC's `/admin/sites`: whether `site_id` is bound there.
async fn bound_at(sbc_http: u16, site_id: &str) -> bool {
    let (code, text) = http_request("GET", &loopback(sbc_http, "/admin/sites"), &[], None)
        .await
        .unwrap();
    assert_eq!(code, 200, "{text}");
    serde_json::from_str::<Vec<serde_json::Value>>(&text)
        .unwrap()
        .iter()
        .any(|s| s["site_id"] == site_id)
}

/// A user made in the cloud, with a softphone and a number answered at
/// the site.
async fn cloud_user(
    cloud: &common::Cloud,
    storage: &fcp_storage_postgres::PostgresStorage,
    username: &str,
    password: &str,
    extension: &str,
    number: Option<&str>,
) {
    use fcp_core::storage::{CredentialStore, DeviceStore, LineBindingStore, PhoneNumberStore};
    let tenant_id: fcp_core::TenantId = cloud.tenant_id.parse().unwrap();
    let (code, made) = cloud
        .admin_call(
            "POST",
            "/api/v1/users",
            true,
            Some(
                &serde_json::json!({
                    "username": username,
                    "password": password,
                    "display_name": username,
                    "extension": extension,
                    "tenant_id": cloud.tenant_id,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    let user = storage
        .get_user_in_tenant(username, &tenant_id)
        .await
        .unwrap()
        .expect("the user is stored");
    let mut device = fcp_core::Device::new(
        format!("{username} softphone"),
        fcp_core::DeviceType::Softphone,
        tenant_id,
    );
    device.owner_user_id = Some(user.id);
    device.sip_uri = Some(format!("sip:{username}@{REALM}"));
    let device_id = device.id;
    storage.put_device(device).await.unwrap();
    // The extension that dials the user, as the seed makes one: the API
    // keeps the user's extension as a field, and the dial plan reads the
    // extensions table.
    {
        use fcp_core::storage::ExtensionStore;
        let ext = fcp_core::Extension::new(
            extension.to_string(),
            fcp_core::ExtensionTarget::User(user.id),
            tenant_id,
        )
        .expect("extension number is valid");
        storage.put_extension(ext).await.unwrap();
    }
    if let Some(e164) = number {
        let mut phone_number = fcp_core::PhoneNumber::new(e164.to_string(), tenant_id).unwrap();
        phone_number.site_id = Some(cloud.site_id.parse().unwrap());
        let mut binding = fcp_core::LineBinding::new(
            phone_number.id,
            Some(user.id),
            fcp_core::BindingType::User,
            tenant_id,
        );
        binding.devices = vec![fcp_core::DeviceBinding::new(device_id)];
        storage.put_phone_number(phone_number).await.unwrap();
        storage.put_line_binding(binding).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_site_cut_off_from_its_cloud_goes_on_alone_and_catches_up_when_the_link_returns() {
    let Some(site_redis) = fcp_testkit::cluster::private_redis_url() else {
        eprintln!("FCP_TEST_REDIS_URL not set; skipping the survivability test");
        return;
    };
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the survivability test");
        return;
    };
    let (events_port, posted) = stub_events_server().await;
    // The SBC's ports first, and the proxy the site reaches its TLS
    // listener through: what the cloud names to the site at enrollment.
    let (sbc_sip, sbc_tls, sbc_http) = (free_port(), free_port(), free_port());
    let sbc_cut = CutLink::open(([127, 0, 0, 1], sbc_tls).into()).await;
    let (cloud_cm_sip, cloud_cm_http) = (free_port(), free_port());
    let (site_cm_sip, site_cm_http) = (free_port(), free_port());
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        sip_host: Some(format!("127.0.0.1:{}", sbc_cut.port())),
        events_server_url: Some(loopback(events_port, "/ingest")),
        site_redis_url: Some(site_redis.clone()),
        cut_able: true,
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let link_cut = cloud
        .link_cut
        .as_ref()
        .expect("the link goes through a proxy");
    let dir = toml_path(cloud.dir.path());
    let tenant_id: fcp_core::TenantId = cloud.tenant_id.parse().unwrap();
    let cloud_store =
        fcp_storage_postgres::PostgresStorage::new(fcp_storage_postgres::PostgresConfig {
            connection_string: cloud.db.url().to_string(),
            read_replica_url: None,
            max_replica_lag: Duration::from_secs(5),
            max_connections: 4,
            connection_timeout: Duration::from_secs(30),
            encryption: None,
        })
        .await
        .expect("the cloud's database");

    // The tenant's carrier in the cloud, and a carrier of the site's own.
    let sbc_sip_addr: SocketAddr = format!("127.0.0.1:{sbc_sip}").parse().unwrap();
    let carrier = Phone::new("carrier", "x", "carrier.example", sbc_sip_addr)
        .await
        .expect("carrier phone binds");
    let nowhere = SocketAddr::from(([127, 0, 0, 1], 9));
    let local_carrier = Phone::new("local-carrier", "x", "carrier.example", nowhere)
        .await
        .expect("local carrier phone binds");
    {
        use fcp_core::storage::TrunkStore;
        let mut carrier_trunk = fcp_core::Trunk::new("carrier".to_string(), tenant_id);
        carrier_trunk.direction = fcp_core::TrunkDirection::Bidirectional;
        carrier_trunk.hosts = vec!["127.0.0.1".to_string()];
        carrier_trunk.transport.protocol = fcp_core::transport::TransportType::Udp;
        carrier_trunk.transport.port = Some(carrier.local_addr.port());
        carrier_trunk.failover.health_check_enabled = false;
        cloud_store.put_trunk(carrier_trunk).await.unwrap();
    }
    let (code, made) = cloud
        .admin_call(
            "POST",
            "/api/v1/trunks",
            true,
            Some(
                &serde_json::json!({
                    "name": "site-carrier",
                    "hosts": ["127.0.0.1"],
                    "transport_protocol": "udp",
                    "transport_port": local_carrier.local_addr.port(),
                    "direction": "bidirectional",
                    "priority": 5,
                    "site_id": cloud.site_id,
                    "health_check_enabled": false,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    assert_eq!(made["site_id"], cloud.site_id, "{made}");

    // Alice and Bob, made in the cloud; Alice's number is answered at the site.
    cloud_user(
        &cloud,
        &cloud_store,
        "alice",
        ALICE_PASSWORD,
        "1001",
        Some(SITE_NUMBER),
    )
    .await;
    cloud_user(&cloud, &cloud_store, "bob", BOB_PASSWORD, "1002", None).await;

    // The cloud's call manager and SBC, the site's call manager.
    let cloud_cm = Service::start_with(
        "fcp-call-manager (cloud)",
        bins::path("fcp-call-manager", "fcp-call-manager"),
        &CallManagerToml {
            node_id: "cloud-cm",
            sip: cloud_cm_sip,
            http: cloud_cm_http,
            db: cloud.db.url(),
            dir: &dir,
            rtp_start: allocate_port_pair().rtp_port,
            ..Default::default()
        }
        .render(),
        &loopback(cloud_cm_http, "/health"),
        &[],
    )
    .await;
    let sbc_cert = cloud.ca.node("fcp-sbc");
    let sbc_rtp = allocate_port_pair().rtp_port;
    let sbc_config = format!(
        r#"[service]
node_id = "cloud-sbc"
drain_wait_secs = 0

[core]
external_domain = "{REALM}"
tls_bind = "127.0.0.1:{sbc_tls}"
tls_cert = "{cert}"
tls_key = "{key}"

[core.storage]
enabled = true
database_url = "{db}"

[core.nat]
rtp_port_start = {sbc_rtp}
rtp_port_end = {sbc_rtp_end}

[[core.trunks]]
name = "call-manager"
enabled = true
direction = "Bidirectional"
hosts = ["127.0.0.1:{cloud_cm_sip}"]
inbound_sources = ["127.0.0.1/32"]
priority = 1
weight = 100
register = false
max_concurrent_calls = 0
"#,
        cert = toml_path(&sbc_cert.cert_path),
        key = toml_path(&sbc_cert.key_path),
        db = cloud.db.url(),
        sbc_rtp_end = sbc_rtp + 60,
    );
    let sbc_bind = format!("127.0.0.1:{sbc_sip}");
    let sbc_health = format!("127.0.0.1:{sbc_http}");
    let sbc = Service::start_with(
        "fcp-sbc",
        bins::path("fcp-sbc", "fcp-sbc"),
        &sbc_config,
        &loopback(sbc_http, "/health"),
        &[
            ("FCP_SBC_BIND", sbc_bind.as_str()),
            ("FCP_SBC_HEALTH_BIND", sbc_health.as_str()),
        ],
    )
    .await;
    let site_cm = Service::start_with(
        "fcp-call-manager (site)",
        bins::path("fcp-call-manager", "fcp-call-manager"),
        &CallManagerToml {
            node_id: "site-cm",
            sip: site_cm_sip,
            http: site_cm_http,
            db: site_db.url(),
            dir: &dir,
            rtp_start: allocate_port_pair().rtp_port,
            tls_ca: Some(&toml_path(&cloud.ca.ca_path)),
            redis_url: Some(&site_redis),
        }
        .render(),
        &loopback(site_cm_http, "/health"),
        &[],
    )
    .await;
    let mut nodes = Nodes {
        cloud_cm,
        sbc,
        site_cm,
    };

    // Whole: the site bound at the cloud, its phones registered.
    let site_id = cloud.site_id.clone();
    check!(
        nodes,
        wait_for(Duration::from_secs(90), || bound_at(sbc_http, &site_id)).await,
        "the SBC never bound the site"
    );
    let site_sip_addr: SocketAddr = format!("127.0.0.1:{site_cm_sip}").parse().unwrap();
    let alice = Phone::new("alice", ALICE_PASSWORD, REALM, site_sip_addr)
        .await
        .unwrap();
    let bob = Phone::new("bob", BOB_PASSWORD, REALM, site_sip_addr)
        .await
        .unwrap();
    registers("alice at the site", &alice).await;
    registers("bob at the site", &bob).await;

    // A carrier's call reaches Alice, and what the cloud sends mid-call —
    // a re-INVITE, then the BYE — goes down the connection the site
    // opened.
    let inbound = carrier.call(SITE_NUMBER).await.unwrap();
    let Some(ringing) = alice.incoming(SOON).await else {
        nodes
            .fail("Alice's phone never rang for the carrier's call")
            .await;
    };
    let alice_leg = ringing.answer().await.unwrap();
    if let Err(e) = inbound.answered(SOON).await {
        nodes
            .fail(&format!("the carrier's call was not answered: {e}"))
            .await;
    }
    let held = inbound.hold().await.unwrap_or(0);
    check!(
        nodes,
        held == 200,
        "the carrier's hold was answered {held}, not 200"
    );
    inbound.hangup().await.unwrap();
    check!(
        nodes,
        alice_leg.ended(SOON).await,
        "the carrier's hangup never reached Alice"
    );

    // The link is cut. The cloud says so; the site is no longer bound.
    link_cut.cut();
    sbc_cut.cut();
    until(
        "the cloud never saw the site's link go down",
        cloud.http,
        &cloud.site_path(),
        Some(&cloud.tenant_id),
        |s| s["link_up"] == false,
    )
    .await;
    check!(
        nodes,
        wait_for(Duration::from_secs(20), || async {
            !bound_at(sbc_http, &site_id).await
        })
        .await,
        "the SBC still lists the site after its connection was cut"
    );

    // The site goes on: phones register, call one another with audio,
    // and reach the outside on the site's own carrier.
    registers("alice re-registering during the cut", &alice).await;
    let local = alice.call("1002").await.unwrap();
    let Some(bob_ringing) = bob.incoming(SOON).await else {
        nodes.fail("Bob's phone never rang during the cut").await;
    };
    let bob_leg = bob_ringing.answer().await.unwrap();
    if let Err(e) = local.answered(SOON).await {
        nodes
            .fail(&format!(
                "Alice's call to Bob was not answered during the cut: {e}"
            ))
            .await;
    }
    local.media().play_tone(440.0, 1500);
    check!(
        nodes,
        bob_leg.media().heard(440, SOON).await,
        "Bob hears Alice during the cut (received {} packets)",
        bob_leg.media().packets_received()
    );
    local.hangup().await.unwrap();
    check!(
        nodes,
        bob_leg.ended(SOON).await,
        "Alice's hangup never reached Bob"
    );
    let outside = alice.call(OUTSIDE_NUMBER).await.unwrap();
    let Some(at_local_carrier) = local_carrier.incoming(SOON).await else {
        nodes
            .fail("the site's own carrier was never asked for Alice's call during the cut")
            .await;
    };
    check!(
        nodes,
        at_local_carrier.request_uri().contains(OUTSIDE_NUMBER),
        "the site's carrier is asked for the number Alice dialled: {}",
        at_local_carrier.request_uri()
    );
    let local_carrier_leg = at_local_carrier.answer().await.unwrap();
    if let Err(e) = outside.answered(SOON).await {
        nodes
            .fail(&format!(
                "Alice's outside call was not answered during the cut: {e}"
            ))
            .await;
    }
    outside.hangup().await.unwrap();
    check!(
        nodes,
        local_carrier_leg.ended(SOON).await,
        "Alice's hangup never reached the site's carrier"
    );

    // Meanwhile the cloud changes its mind: Carol is made.
    cloud_user(&cloud, &cloud_store, "carol", CAROL_PASSWORD, "1003", None).await;
    let before_restore = cloud_store
        .query_cdrs(CdrQuery {
            tenant_id: Some(tenant_id),
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .filter(|r| r.site_id.is_some())
        .count();

    // The link returns. The cloud says the site is back; what the cloud
    // changed reaches the site; what the site wrote reaches the cloud;
    // the site registers its link again and the cloud's calls reach it.
    link_cut.restore().await;
    sbc_cut.restore().await;
    until(
        "the cloud never saw the site's link come back",
        cloud.http,
        &cloud.site_path(),
        Some(&cloud.tenant_id),
        |s| s["link_up"] == true,
    )
    .await;
    let carol = Phone::new("carol", CAROL_PASSWORD, REALM, site_sip_addr)
        .await
        .unwrap();
    let carol_registered = wait_for(Duration::from_secs(60), || async {
        carol.register().await.unwrap_or(0) == 200
    })
    .await;
    if !carol_registered {
        let (_, status) = http_request("GET", &loopback(cloud.link_health, "/status"), &[], None)
            .await
            .unwrap_or_default();
        nodes
            .fail(&format!(
                "Carol, made during the cut, never registered at the site after the link returned; the link says {status}"
            ))
            .await;
    }
    eventually(
        "the site's records reaching the cloud",
        Duration::from_secs(60),
        || async {
            let records = cloud_store
                .query_cdrs(CdrQuery {
                    tenant_id: Some(tenant_id),
                    ..Default::default()
                })
                .await
                .unwrap();
            let from_site: Vec<_> = records.iter().filter(|r| r.site_id.is_some()).collect();
            from_site.len() >= before_restore + 2
                && from_site.iter().any(|r| r.to_uri.contains(OUTSIDE_NUMBER))
        },
    )
    .await;
    eventually(
        "the site's events reaching the cloud",
        Duration::from_secs(60),
        || async {
            posted.lock().unwrap().iter().any(|p| {
                p.path == "/ingest/batch"
                    && p.origin_site.as_deref() == Some(cloud.site_id.as_str())
                    && p.body
                        .as_array()
                        .is_some_and(|events| events.iter().any(|e| e["event_type"] == "call"))
            })
        },
    )
    .await;
    check!(
        nodes,
        wait_for(Duration::from_secs(120), || bound_at(sbc_http, &site_id)).await,
        "the site never registered its link again"
    );
    let again = carrier.call(SITE_NUMBER).await.unwrap();
    let Some(ringing) = alice.incoming(SOON).await else {
        nodes
            .fail("Alice's phone never rang after the link returned")
            .await;
    };
    let alice_leg = ringing.answer().await.unwrap();
    if let Err(e) = again.answered(SOON).await {
        nodes
            .fail(&format!(
                "the carrier's call after the link returned was not answered: {e}"
            ))
            .await;
    }
    again.hangup().await.unwrap();
    check!(
        nodes,
        alice_leg.ended(SOON).await,
        "the hangup after the link returned never reached Alice"
    );

    let site_log = nodes.site_cm.stop().await;
    assert!(
        !site_log.contains("panicked"),
        "the site's call manager panicked:\n{site_log}"
    );
    let sbc_log = nodes.sbc.stop().await;
    assert!(
        !sbc_log.contains("panicked"),
        "the SBC panicked:\n{sbc_log}"
    );
    assert!(
        sbc_log.contains("Forwarding re-INVITE via RFC 5626 flow"),
        "the carrier's hold reached the site down its flow:\n{sbc_log}"
    );
    assert!(
        sbc_log.contains("Sending BYE via RFC 5626 flow"),
        "the carrier's hangup reached the site down its flow:\n{sbc_log}"
    );
    let cloud_log = nodes.cloud_cm.stop().await;
    assert!(
        !cloud_log.contains("panicked"),
        "the cloud's call manager panicked:\n{cloud_log}"
    );
    cloud.finish().await;
    site_db.drop_db().await;
}

/// Wait for `check` to hold, up to `within`.
async fn wait_for<F, Fut>(within: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The cloud's and the site's services, stopped together with their logs
/// shown when something before the end went wrong.
struct Nodes {
    cloud_cm: Service,
    sbc: Service,
    site_cm: Service,
}

impl Nodes {
    async fn fail(self, what: &str) -> ! {
        let mut logs = String::new();
        logs.push_str("\n===== the site's call manager =====\n");
        logs.push_str(&self.site_cm.kill().await);
        logs.push_str("\n===== the SBC =====\n");
        logs.push_str(&self.sbc.kill().await);
        logs.push_str("\n===== the cloud's call manager =====\n");
        logs.push_str(&self.cloud_cm.kill().await);
        panic!("{what}\n{logs}");
    }
}
