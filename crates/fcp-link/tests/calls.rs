//! Calls across a site's link (`docs/HYBRID_CONTROL_PLANE.md` phase 4):
//! the cloud gives the site a link trunk at enrollment, the site's call
//! manager registers it at the cloud SBC over TLS, a carrier's call to a
//! number at the site goes down that registration and rings the site's
//! phone with audio both ways, a site phone's call to the outside leaves
//! through the cloud — the SBC, then the cloud's call manager as the
//! tenant's own call, then the tenant's carrier — and a site whose call
//! manager is gone is unbound, so its numbers answer 480 rather than
//! ring nothing. A site phone's 911 leaves through the cloud too, placed
//! where the site says its caller is: the PIDF-LO the site resolved goes
//! up the link and on to the carrier, with the site's ELIN as the caller. Gated on `FCP_TEST_DATABASE_URL`; skipped with a notice
//! when unset.

mod common;

use common::{cloud_with_site_opts, CloudOptions};
use fcp_core::storage::{
    CredentialStore, DeviceStore, LineBindingStore, PhoneNumberStore, SiteLocationStore,
};
use fcp_testkit::media::allocate_port_pair;
use fcp_testkit::smoke::{free_port, http_request, loopback, toml_path};
use fcp_testkit::{bins, Phone, Service, TestDatabase};
use std::net::SocketAddr;
use std::time::Duration;

/// The cloud's realm — what its API computes digest credentials for and
/// its SBC challenges with — and the site's, since the tenant is the same.
const REALM: &str = "fcp.local";
/// A number of the tenant's, answered at the site.
const SITE_NUMBER: &str = "+15125550111";
/// A number of nobody's here: the outside, reached through the carrier.
const OUTSIDE_NUMBER: &str = "+15125550999";
const ALICE_PASSWORD: &str = "Alice-at-the-site-2026!";
/// The callback number (ELIN) of Alice's office, which the carrier maps
/// to its address.
const OFFICE_ELIN: &str = "+15125550123";
const SOON: Duration = Duration::from_secs(10);

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

/// A call manager's TOML: the cloud's, or the site's.
#[allow(clippy::too_many_arguments)]
fn call_manager_toml(
    node_id: &str,
    sip: u16,
    http: u16,
    db: &str,
    dir: &str,
    rtp_start: u16,
    tls_ca: Option<&str>,
) -> String {
    let tls_ca = match tls_ca {
        Some(path) => format!("tls_ca_path = \"{path}\"\n"),
        None => String::new(),
    };
    format!(
        r#"[service]
node_id = "{node_id}"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"
{tls_ca}
[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db}"

[cdr]
spool_dir = "{dir}/{node_id}-cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

[calls]
call_timeout_seconds = 8

[b2bua.forking]
timeout = 8

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}
prompts_dir = "{dir}/{node_id}-prompts"
tts_cache_dir = "{dir}/{node_id}-tts-cache"
recording_dir = "{dir}/{node_id}-recordings"

[trunk_routing]
refresh_secs = 1

# The cloud SBC stands in front of this node and names the site a call
# came from; the site's phones and the test's carrier are on the loopback.
[e911]
trusted_edges = ["127.0.0.0/8"]

[logging]
level = "info"
format = "compact"
"#,
        rtp_end = rtp_start + 60,
    )
}

/// The SBC's `/admin/sites`: the sites bound there.
async fn bound_sites(sbc_http: u16) -> Vec<serde_json::Value> {
    let (code, text) = http_request("GET", &loopback(sbc_http, "/admin/sites"), &[], None)
        .await
        .unwrap();
    assert_eq!(code, 200, "{text}");
    serde_json::from_str::<Vec<serde_json::Value>>(&text).unwrap()
}

/// Wait until the SBC lists `site_id` as bound (or, with `bound` false,
/// no longer does); what went wrong otherwise.
async fn site_bound_at(
    sbc_http: u16,
    site_id: &str,
    bound: bool,
    within: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let sites = bound_sites(sbc_http).await;
        let is_bound = sites.iter().any(|s| s["site_id"] == site_id);
        if is_bound == bound {
            if bound {
                let site = sites.iter().find(|s| s["site_id"] == site_id).unwrap();
                if site["via_flow"] != true {
                    return Err(format!(
                        "the site is not reached down the connection it opened: {site}"
                    ));
                }
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "the SBC {} the site after {within:?}: {sites:?}",
                if bound { "never bound" } else { "still lists" }
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// `assert!` that shows every node's log when it fails.
macro_rules! check {
    ($nodes:expr, $cond:expr, $($arg:tt)+) => {
        if !$cond {
            $nodes.fail(&format!($($arg)+)).await;
        }
    };
}

/// The cloud's and the site's services, stopped together with their
/// logs shown when something before the end went wrong.
struct Nodes {
    cloud_cm: Service,
    sbc: Service,
    site_cm: Option<Service>,
}

impl Nodes {
    async fn fail(self, what: &str) -> ! {
        let mut logs = String::new();
        if let Some(site_cm) = self.site_cm {
            logs.push_str("\n===== the site's call manager =====\n");
            logs.push_str(&site_cm.kill().await);
        }
        logs.push_str("\n===== the SBC =====\n");
        logs.push_str(&self.sbc.kill().await);
        logs.push_str("\n===== the cloud's call manager =====\n");
        logs.push_str(&self.cloud_cm.kill().await);
        panic!("{what}\n{logs}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calls_cross_a_sites_link_both_ways() {
    let Some(site_db) = TestDatabase::create().await else {
        eprintln!("FCP_TEST_DATABASE_URL not set; skipping the calls test");
        return;
    };
    // The SBC's ports first: the cloud names its TLS address to the site
    // at enrollment.
    let (sbc_sip, sbc_tls, sbc_http) = (free_port(), free_port(), free_port());
    let (cloud_cm_sip, cloud_cm_http) = (free_port(), free_port());
    let (site_cm_sip, site_cm_http) = (free_port(), free_port());
    let cloud = cloud_with_site_opts(CloudOptions {
        site_database_url: Some(site_db.url()),
        sip_host: Some(format!("127.0.0.1:{sbc_tls}")),
        ..Default::default()
    })
    .await
    .expect("the database was there a moment ago");
    let dir = toml_path(cloud.dir.path());
    let tenant_id: fcp_core::TenantId = cloud.tenant_id.parse().unwrap();
    let site_id: fcp_core::SiteId = cloud.site_id.parse().unwrap();

    // Enrollment made the site's link trunk, the cloud's end of it.
    let (code, site) = cloud
        .admin_call("GET", &cloud.site_path(), true, None)
        .await;
    assert_eq!(code, 200, "{site}");
    let trunk_id = site["trunk_id"]
        .as_str()
        .expect("the site has a link trunk")
        .to_string();
    let (code, trunk) = cloud
        .admin_call("GET", &format!("/api/v1/trunks/{trunk_id}"), true, None)
        .await;
    assert_eq!(code, 200, "{trunk}");
    assert_eq!(trunk["site_id"], cloud.site_id, "{trunk}");
    assert_eq!(trunk["link"], "cloud", "{trunk}");
    assert_eq!(trunk["transport_protocol"], "tls", "{trunk}");
    assert_eq!(trunk["register"], true, "{trunk}");
    assert_eq!(trunk["has_credentials"], true, "{trunk}");

    // The tenant's carrier, on the loopback, that answers whatever the
    // cloud sends it and places a carrier's calls at the SBC.
    let sbc_sip_addr: SocketAddr = format!("127.0.0.1:{sbc_sip}").parse().unwrap();
    let carrier = Phone::new("carrier", "x", "carrier.example", sbc_sip_addr)
        .await
        .expect("carrier phone binds");
    let storage =
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
    {
        use fcp_core::storage::TrunkStore;
        let mut carrier_trunk = fcp_core::Trunk::new("carrier".to_string(), tenant_id);
        carrier_trunk.enabled = true;
        carrier_trunk.direction = fcp_core::TrunkDirection::Bidirectional;
        carrier_trunk.hosts = vec!["127.0.0.1".to_string()];
        carrier_trunk.transport.protocol = fcp_core::transport::TransportType::Udp;
        carrier_trunk.transport.port = Some(carrier.local_addr.port());
        carrier_trunk.failover.health_check_enabled = false;
        // The tenant's emergency carrier too, taking the location by value.
        carrier_trunk.emergency_capable = true;
        carrier_trunk.emergency_location = vec![
            fcp_core::e911::LocationDelivery::PidfLo,
            fcp_core::e911::LocationDelivery::Elin,
        ];
        storage
            .put_trunk(carrier_trunk)
            .await
            .expect("the carrier trunk");
    }

    // Alice: a user made in the cloud, with a softphone, whose number is
    // answered at the site.
    let (code, made) = cloud
        .admin_call(
            "POST",
            "/api/v1/users",
            true,
            Some(
                &serde_json::json!({
                    "username": "alice",
                    "password": ALICE_PASSWORD,
                    "display_name": "Alice",
                    "tenant_id": cloud.tenant_id,
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(code, 201, "{made}");
    let alice_user = storage
        .get_user_in_tenant("alice", &tenant_id)
        .await
        .unwrap()
        .expect("alice is stored");
    let mut device = fcp_core::Device::new(
        "alice softphone".to_string(),
        fcp_core::DeviceType::Softphone,
        tenant_id,
    );
    device.owner_user_id = Some(alice_user.id);
    device.sip_uri = Some(format!("sip:alice@{REALM}"));
    // Where Alice's phone is: an office of the tenant's, synced to the
    // site with the rest of the tenant's E911 places.
    let mut office = fcp_core::SiteLocation::new("Austin office".to_string(), tenant_id);
    office.address_line1 = Some("600 Congress Ave".to_string());
    office.city = Some("Austin".to_string());
    office.state = Some("TX".to_string());
    office.postal_code = Some("78701".to_string());
    office.country = Some("US".to_string());
    office.e911_callback_number = Some(OFFICE_ELIN.to_string());
    device.location_id = Some(office.id);
    storage.put_location(office).await.expect("alice's office");
    let device_id = device.id;
    storage.put_device(device).await.expect("alice's device");
    let mut number = fcp_core::PhoneNumber::new(SITE_NUMBER.to_string(), tenant_id).unwrap();
    number.site_id = Some(site_id);
    let mut binding = fcp_core::LineBinding::new(
        number.id,
        Some(alice_user.id),
        fcp_core::BindingType::User,
        tenant_id,
    );
    binding.devices = vec![fcp_core::DeviceBinding::new(device_id)];
    storage.put_phone_number(number).await.expect("the number");
    storage
        .put_line_binding(binding)
        .await
        .expect("the binding");
    let (code, listed) = cloud
        .admin_call("GET", "/api/v1/phone-numbers", true, None)
        .await;
    assert_eq!(code, 200, "{listed}");
    let numbers = listed["data"]
        .as_array()
        .or_else(|| listed.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        numbers
            .iter()
            .any(|n| n["e164"] == SITE_NUMBER && n["site_id"] == cloud.site_id),
        "the number says where it is answered: {listed}"
    );

    // The cloud's call manager: the tenant's own calls, and its carrier.
    let cloud_cm = Service::start_with(
        "fcp-call-manager (cloud)",
        bins::path("fcp-call-manager", "fcp-call-manager"),
        &call_manager_toml(
            "cloud-cm",
            cloud_cm_sip,
            cloud_cm_http,
            cloud.db.url(),
            &dir,
            allocate_port_pair().rtp_port,
            None,
        ),
        &loopback(cloud_cm_http, "/health"),
        &[],
    )
    .await;

    // The cloud SBC: TLS from the test's CA, the cloud's database, the
    // call manager behind it as its one trunk.
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

    // The site's call manager, on the site's database, trusting the
    // cloud's CA for the link.
    let site_cm = Service::start_with(
        "fcp-call-manager (site)",
        bins::path("fcp-call-manager", "fcp-call-manager"),
        &call_manager_toml(
            "site-cm",
            site_cm_sip,
            site_cm_http,
            site_db.url(),
            &dir,
            allocate_port_pair().rtp_port,
            Some(&toml_path(&cloud.ca.ca_path)),
        ),
        &loopback(site_cm_http, "/health"),
        &[],
    )
    .await;

    let mut nodes = Nodes {
        cloud_cm,
        sbc,
        site_cm: Some(site_cm),
    };

    // The site registers its link at the SBC, down the connection it
    // opened — after the link has synced the trunk to the site and the
    // SBC has read it.
    if let Err(e) = site_bound_at(sbc_http, &cloud.site_id, true, Duration::from_secs(90)).await {
        nodes.fail(&e).await;
    }

    // Alice's phone registers at the site, with the password the cloud set.
    let site_sip_addr: SocketAddr = format!("127.0.0.1:{site_cm_sip}").parse().unwrap();
    let alice = Phone::new("alice", ALICE_PASSWORD, REALM, site_sip_addr)
        .await
        .unwrap();
    registers("alice at the site", &alice).await;

    // A carrier's call to the site's number reaches Alice, with audio both
    // ways through the cloud SBC and the site's call manager.
    let inbound = carrier.call(SITE_NUMBER).await.unwrap();
    let Some(ringing) = alice.incoming(SOON).await else {
        nodes
            .fail("Alice's phone never rang for the carrier's call")
            .await;
    };
    check!(
        nodes,
        ringing.request_uri().contains("alice"),
        "the site rings Alice's own contact: {}",
        ringing.request_uri()
    );
    let alice_leg = ringing.answer().await.unwrap();
    if let Err(e) = inbound.answered(SOON).await {
        nodes
            .fail(&format!("the carrier's call was not answered: {e}"))
            .await;
    }
    alice_leg.media().play_tone(440.0, 1500);
    check!(
        nodes,
        inbound.media().heard(440, SOON).await,
        "the carrier hears Alice's 440 Hz (received {} packets)",
        inbound.media().packets_received()
    );
    inbound.media().play_tone(880.0, 1500);
    check!(
        nodes,
        alice_leg.media().heard(880, SOON).await,
        "Alice hears the carrier's 880 Hz (received {} packets)",
        alice_leg.media().packets_received()
    );
    inbound.hangup().await.unwrap();
    check!(
        nodes,
        alice_leg.ended(SOON).await,
        "the carrier's hangup reaches Alice down the site's link"
    );

    // Alice's call to the outside leaves through the cloud: the SBC, the
    // cloud's call manager as the tenant's own call, the tenant's carrier.
    let outbound = alice.call(OUTSIDE_NUMBER).await.unwrap();
    let Some(at_carrier) = carrier.incoming(SOON).await else {
        nodes
            .fail("the carrier was never asked for Alice's call")
            .await;
    };
    check!(
        nodes,
        at_carrier.request_uri().contains(OUTSIDE_NUMBER),
        "the carrier is asked for the number Alice dialled: {}",
        at_carrier.request_uri()
    );
    let from = at_carrier.header("From").unwrap_or_default();
    check!(
        nodes,
        from.contains("alice"),
        "the call is Alice's, as far as the carrier sees: {from}"
    );
    let carrier_leg = at_carrier.answer().await.unwrap();
    if let Err(e) = outbound.answered(SOON).await {
        nodes
            .fail(&format!("Alice's call was not answered: {e}"))
            .await;
    }
    outbound.media().play_tone(440.0, 1500);
    check!(
        nodes,
        carrier_leg.media().heard(440, SOON).await,
        "the carrier hears Alice's 440 Hz (received {} packets)",
        carrier_leg.media().packets_received()
    );
    carrier_leg.media().play_tone(880.0, 1500);
    check!(
        nodes,
        outbound.media().heard(880, SOON).await,
        "Alice hears the carrier's 880 Hz (received {} packets)",
        outbound.media().packets_received()
    );
    outbound.hangup().await.unwrap();
    check!(
        nodes,
        carrier_leg.ended(SOON).await,
        "Alice's hangup reaches the carrier"
    );

    // The cloud's call manager carried Alice's call as the tenant's own:
    // its record is the tenant's, out on the tenant's carrier.
    {
        use fcp_core::storage::{CdrQuery, CdrStore};
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let records = storage
                .query_cdrs(CdrQuery {
                    tenant_id: Some(tenant_id),
                    ..Default::default()
                })
                .await
                .unwrap();
            let outside = records
                .iter()
                .filter(|r| r.to_uri.contains(OUTSIDE_NUMBER))
                .collect::<Vec<_>>();
            if outside
                .iter()
                .any(|r| r.trunk_name.as_deref() == Some("carrier"))
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no record of Alice's call out on the tenant's carrier: {records:#?}"
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    // Alice's 911 at the site, which has no emergency carrier of its own,
    // leaves up the link and on to the tenant's carrier, placed where the
    // site says she is: her office, by value, with its ELIN as the caller.
    let emergency = alice.call("911").await.unwrap();
    let Some(at_psap) = carrier.incoming(SOON).await else {
        nodes
            .fail("the carrier was never asked for Alice's 911")
            .await;
    };
    let location = at_psap
        .multipart()
        .and_then(|m| {
            m.part_by_type(fcp_core::pidf_lo::CONTENT_TYPE)
                .map(|p| String::from_utf8_lossy(p.body()).into_owned())
        })
        .and_then(|xml| fcp_core::pidf_lo::parse(&xml))
        .and_then(|parsed| parsed.address);
    check!(
        nodes,
        location
            .as_ref()
            .is_some_and(|a| a.address1.contains("600") && a.city == "Austin"),
        "the carrier is told where Alice is, as the site resolved it: {location:?}"
    );
    check!(
        nodes,
        at_psap.header("Geolocation").is_some(),
        "the location is named by Geolocation"
    );
    let caller = format!(
        "{} {}",
        at_psap.header("From").unwrap_or_default(),
        at_psap.header("P-Asserted-Identity").unwrap_or_default()
    );
    check!(
        nodes,
        caller.contains(OFFICE_ELIN),
        "the PSAP calls back the office's ELIN: {caller}"
    );
    let psap_leg = at_psap.answer().await.unwrap();
    if let Err(e) = emergency.answered(SOON).await {
        nodes
            .fail(&format!("Alice's 911 was not answered: {e}"))
            .await;
    }
    emergency.hangup().await.unwrap();
    check!(
        nodes,
        psap_leg.ended(SOON).await,
        "Alice's hangup reaches the PSAP"
    );

    // The site's call manager gone, its connection closes and the site is
    // unbound: a carrier's call to its number is refused, not rung into
    // nothing.
    let site_log = nodes.site_cm.take().unwrap().kill().await;
    check!(
        nodes,
        !site_log.contains("panicked"),
        "the site's call manager panicked:\n{site_log}"
    );
    if let Err(e) = site_bound_at(sbc_http, &cloud.site_id, false, Duration::from_secs(20)).await {
        nodes.fail(&e).await;
    }
    let refused = carrier.call(SITE_NUMBER).await.unwrap();
    let outcome = refused.failed(SOON).await;
    check!(
        nodes,
        outcome == Some(480),
        "a number at a site that is not bound is temporarily unavailable, not {outcome:?}"
    );

    let sbc_log = nodes.sbc.stop().await;
    assert!(
        !sbc_log.contains("panicked"),
        "the SBC panicked:\n{sbc_log}"
    );
    let cloud_log = nodes.cloud_cm.stop().await;
    assert!(
        !cloud_log.contains("panicked"),
        "the cloud's call manager panicked:\n{cloud_log}"
    );
    cloud.finish().await;
    site_db.drop_db().await;
}
