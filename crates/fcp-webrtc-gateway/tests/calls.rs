//! Emergency calls from a browser through the WebRTC gateway
//! (`docs/E911_BLOCK_2.md` §4 and §10): an emergency number is placed only
//! for a named tenant user and only through the call manager, which
//! believes the gateway's caller headers because the gateway is one of its
//! trusted edges, locates the user and sends the call to the emergency
//! trunk with their site's callback number. Without a caller, or without a
//! call manager to go through, the gateway refuses with the reason. The
//! refusals need no database; the located call is skipped without
//! `FCP_TEST_DATABASE_URL`.

use fcp_testkit::media::allocate_rtp_range;
use fcp_testkit::smoke::{http_request, loopback};
use fcp_testkit::{bins, free_port, Phone, Seed, Service, TestDatabase};
use forge_core::AudioCodec;
use forge_webrtc::{ConnectionState, PeerConfig, PeerConnection, PeerEvent};
use std::net::SocketAddr;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-webrtc-gw");
const REALM: &str = "e2e.local";
const OPERATOR: &str = "e2e-operator";
const ELIN: &str = "+15185550100";
const BRANCH_ELIN: &str = "+15185550222";
const SOON: Duration = Duration::from_secs(10);
const A_WHILE: Duration = Duration::from_secs(20);

struct Gateway {
    service: Option<Service>,
    http: u16,
    _dir: tempfile::TempDir,
}

impl Gateway {
    /// A gateway sending calls through `call_manager`, when there is one.
    async fn start(call_manager: Option<SocketAddr>) -> Self {
        let sip = free_port();
        let http = free_port();
        let rtp_start = 20000 + (uuid::Uuid::new_v4().as_u128() % 20) as u16 * 400;
        let dir = tempfile::tempdir().expect("tempdir");
        let proxy = call_manager
            .map(|cm| format!("outbound_proxy = \"{cm}\""))
            .unwrap_or_default();
        let config = format!(
            r#"
[service]
node_id = "e2e-webrtc-gw"
shutdown_timeout_secs = 5

[http]
bind_address = "127.0.0.1:{http}"

[auth]
tokens = ["{OPERATOR}"]

[sip]
# The gateway is an edge of its own: a different address from the browsers
# it fronts, so the call manager can tell them apart.
udp_bind = "127.0.0.3:{sip}"
local_ip = "127.0.0.3"
{proxy}

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[webrtc]
stun_servers = []

[conference]
server_uri = "sip:conference@127.0.0.1:9"
srtp = false

[tls]
enabled = false

[logging]
level = "info"
format = "compact"
"#,
            rtp_end = rtp_start + 400,
        );
        let service = Service::start("webrtc-gw", BIN, &config, &loopback(http, "/health")).await;
        Self {
            service: Some(service),
            http,
            _dir: dir,
        }
    }

    async fn api(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<&str>,
    ) -> (u16, serde_json::Value) {
        let auth = format!("Bearer {token}");
        let (code, body) = http_request(
            method,
            &loopback(self.http, path),
            &[("Authorization", &auth)],
            body,
        )
        .await
        .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
        let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
        (code, json)
    }

    async fn call(&self, body: serde_json::Value) -> (u16, serde_json::Value) {
        self.api("POST", "/v1/calls", OPERATOR, Some(&body.to_string()))
            .await
    }

    async fn logs(&mut self) -> String {
        match self.service.take() {
            Some(s) => s.kill().await,
            None => String::new(),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_emergency_number_is_refused_without_a_caller_or_a_call_manager() {
    let caller = serde_json::json!({
        "tenant_id": fcp_core::TenantId::nil().to_string(),
        "username": "alice",
    });

    // With a call manager to go through, a call with no caller is refused,
    // prefixed or not, live or test.
    let mut gw = Gateway::start(Some(SocketAddr::from(([127, 0, 0, 1], 9)))).await;
    for number in ["911", "9911", "933", "sip:911@carrier.example"] {
        let (code, body) = gw.call(serde_json::json!({ "destination": number })).await;
        assert_eq!(code, 403, "{number}: {body}");
        assert!(
            body["error"].as_str().unwrap_or("").contains("caller"),
            "{number}: the reason names what is missing: {body}"
        );
    }
    // A caller that could not be put in a header is refused before that.
    let (code, body) = gw
        .call(serde_json::json!({
            "destination": "911",
            "caller": { "tenant_id": fcp_core::TenantId::nil().to_string(), "username": "a b" },
        }))
        .await;
    assert_eq!(code, 400, "{body}");
    let (listed, sessions) = gw.api("GET", "/v1/calls", OPERATOR, None).await;
    assert_eq!(listed, 200);
    assert_eq!(sessions["count"], 0, "no session was created: {sessions}");
    gw.logs().await;

    // Without one, even a named caller's emergency call is refused.
    let mut gw = Gateway::start(None).await;
    let (code, body) = gw
        .call(serde_json::json!({ "destination": "911", "caller": caller }))
        .await;
    assert_eq!(code, 503, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("outbound_proxy"),
        "{body}"
    );
    gw.logs().await;
}

/// A call manager whose emergency trunk is `carrier`, trusting loopback as
/// an edge.
async fn call_manager(
    db_url: &str,
    carrier: &Phone,
    dir: &std::path::Path,
) -> (Service, SocketAddr) {
    let sip = free_port();
    let http = free_port();
    let rtp_start = allocate_rtp_range(60);
    let config = format!(
        r#"
[service]
node_id = "e2e-call-manager"

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
local_ip = "127.0.0.1"

[registrar]
auth_enabled = true
realm = "{REALM}"

[storage]
database_url = "{db_url}"

[cdr]
spool_dir = "{dir}/cdr-spool"

[http]
bind_address = "127.0.0.1:{http}"

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
trusted_edges = ["127.0.0.3"]

[[trunks]]
name = "psap"
enabled = true
direction = "Upstream"
hosts = ["127.0.0.1"]
weight = 100
priority = 1
register = false
max_concurrent_calls = 0
emergency_capable = true

[trunks.transport]
protocol = "Udp"
port = {carrier}
verify_tls = false

[trunks.failover]
max_failures = 3
recovery_interval_secs = 30
health_check_enabled = false
health_check_interval_secs = 60
"#,
        rtp_end = rtp_start + 60,
        dir = dir.display(),
        carrier = carrier.local_addr.port(),
    );
    let bin = bins::path("fcp-call-manager", "fcp-call-manager");
    let service = Service::start("call-manager", bin, &config, &loopback(http, "/health")).await;
    (service, SocketAddr::from(([127, 0, 0, 1], sip)))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_emergency_call_is_located_through_the_call_manager() {
    let Some(db) = TestDatabase::create().await else {
        eprintln!("skipped: FCP_TEST_DATABASE_URL is not set");
        return;
    };
    let seed = Seed::new(db.url(), REALM).await;
    seed.user("alice", "alice-pw", "1001").await;
    seed.site_for("alice", ELIN).await;
    // The browser's own address is at the branch office; the gateway says
    // where the browser is, and the call manager places it there rather
    // than at the site Alice is provisioned at.
    branch_network(&seed).await;
    let carrier = Phone::new("psap", "x", REALM, SocketAddr::from(([127, 0, 0, 1], 9)))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (cm, cm_sip) = call_manager(db.url(), &carrier, dir.path()).await;
    let mut gw = Gateway::start(Some(cm_sip)).await;

    // The operator places a browser's call to 911 for Alice.
    let (code, created) = gw
        .call(serde_json::json!({
            "destination": "911",
            "caller": { "tenant_id": seed.tenant_id.to_string(), "username": "alice" },
        }))
        .await;
    assert_eq!(code, 201, "{created}");
    assert_eq!(created["emergency"], "live", "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let token = created["session_token"].as_str().unwrap().to_string();
    let offer = created["sdp_offer"].as_str().unwrap().to_string();

    // The browser answers and connects; only then does the gateway dial.
    let mut peer = PeerConnection::with_config(PeerConfig {
        codecs: vec![(AudioCodec::PCMU, 0)],
        video: None,
        ..PeerConfig::default()
    })
    .await
    .unwrap();
    peer.set_remote_offer(&offer).await.unwrap();
    let answer = peer.create_answer().await.unwrap();
    let mut events = peer.take_events().unwrap();
    let (code, body) = gw
        .api(
            "POST",
            &format!("/v1/calls/{id}/answer"),
            &token,
            Some(&serde_json::json!({ "sdp_answer": answer }).to_string()),
        )
        .await;
    assert!((200..300).contains(&code), "answer: {code} {body}");
    let deadline = tokio::time::Instant::now() + A_WHILE;
    while peer.get_state() != ConnectionState::Connected {
        tokio::select! {
            Some(PeerEvent::LocalCandidate(c)) = events.recv() => {
                let body = serde_json::json!({ "candidate": c.to_sdp_attribute() }).to_string();
                gw.api("POST", &format!("/v1/calls/{id}/ice-candidate"), &token, Some(&body)).await;
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            let logs = gw.logs().await;
            panic!("the browser never connected\n{logs}");
        }
    }

    // The emergency trunk receives Alice's site's callback number.
    let Some(incoming) = carrier.incoming(A_WHILE).await else {
        let gw_logs = gw.logs().await;
        let cm_logs = cm.kill().await;
        panic!("the emergency trunk never rang\n{gw_logs}\n{cm_logs}");
    };
    assert!(
        incoming.from.contains(BRANCH_ELIN),
        "the browser is placed by the network it is on: {}",
        incoming.from
    );
    assert_eq!(incoming.header("Priority").as_deref(), Some("emergency"));
    assert_eq!(
        incoming.header("P-Asserted-Identity").as_deref(),
        Some(format!("<sip:{BRANCH_ELIN}@127.0.0.3>").as_str())
    );
    assert!(
        incoming.header(fcp_core::CALLER_USER_HEADER).is_none(),
        "the caller headers stop at the call manager"
    );
    incoming.reject(486, "Busy Here").await.unwrap();

    // The record names Alice, whom only the gateway's headers named.
    let records = seed
        .emergency_calls(1, SOON, |r| r.ended_at.is_some())
        .await;
    assert_eq!(records[0].caller_username.as_deref(), Some("alice"));
    assert_eq!(records[0].callback_number.as_deref(), Some(BRANCH_ELIN));
    let location = records[0].location.as_ref().expect("a location");
    assert_eq!(location.source, fcp_core::e911::LocationSource::Network);
    assert_eq!(
        location.client_address.as_deref(),
        Some("127.0.0.1"),
        "the address the gateway vouched for"
    );
    assert_eq!(records[0].trunk_name.as_deref(), Some("psap"));

    peer.close();
    gw.logs().await;
    cm.stop().await;
    db.drop_db().await;
}

/// A site for the network the browser is on, so a browser at 127.0.0.1 is
/// placed there rather than at the site its user is provisioned at.
async fn branch_network(seed: &Seed) {
    use fcp_core::e911::EmergencyStore;
    use fcp_core::storage::SiteLocationStore;

    let mut site = fcp_core::SiteLocation::new("Branch".to_string(), seed.tenant_id);
    site.address_line1 = Some("9 Elm St".to_string());
    site.city = Some("Troy".to_string());
    site.state = Some("NY".to_string());
    site.postal_code = Some("12180".to_string());
    site.country = Some("US".to_string());
    site.e911_callback_number = Some(BRANCH_ELIN.to_string());
    let location_id = site.id;
    seed.storage().put_location(site).await.unwrap();

    let store =
        fcp_storage_postgres::emergency::PostgresEmergencyStore::new(seed.storage().pool().clone());
    let network = fcp_core::e911::EmergencyNetwork::new(
        seed.tenant_id,
        "127.0.0.1/32".parse().unwrap(),
        location_id,
    );
    store.put_emergency_network(&network).await.unwrap();
}
