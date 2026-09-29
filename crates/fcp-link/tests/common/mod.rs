//! A cloud and a site for the link tests: an admin with its link listeners
//! under the testkit's CA, a tenant and a site made through the API, and
//! `fcp-link` enrolled and heartbeating. Gated by the caller on
//! `FCP_TEST_DATABASE_URL`.

#![allow(dead_code)]

use fcp_testkit::certs::ClusterCa;
use fcp_testkit::smoke::{free_port, http_request, loopback, toml_path, trust_ca};
use fcp_testkit::{bins, Service, TestDatabase};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the admin authenticates to the events server ingest with.
pub const EVENTS_TOKEN: &str = "events-ingest-token-change-me-32-chars";
pub const ADMIN_TOKEN: &str = "link-test-admin-token-change-me-32";
pub const A_WHILE: Duration = Duration::from_secs(20);

pub struct Cloud {
    pub db: TestDatabase,
    pub dir: tempfile::TempDir,
    pub ca: ClusterCa,
    pub http: u16,
    pub enroll_port: u16,
    pub link_port: u16,
    pub link_health: u16,
    pub admin: Option<Service>,
    pub link: Option<Service>,
    pub tenant_id: String,
    pub site_id: String,
    pub token: String,
    /// The proxy the link reaches the cloud through, when asked for one.
    pub link_cut: Option<fcp_testkit::cut::CutLink>,
}

impl Cloud {
    pub fn site_path(&self) -> String {
        format!("/api/v1/sites/{}", self.site_id)
    }

    pub fn state_dir(&self) -> PathBuf {
        self.dir.path().join("link-state")
    }

    pub async fn admin_call(
        &self,
        method: &str,
        path: &str,
        tenant: bool,
        body: Option<&str>,
    ) -> (u16, serde_json::Value) {
        admin(
            self.http,
            method,
            path,
            tenant.then_some(self.tenant_id.as_str()),
            body,
        )
        .await
    }

    /// A client that is the site: the link's certificate and key from its
    /// state directory, trusting the cloud's CA.
    pub fn site_client(&self) -> reqwest::Client {
        let mut pem = std::fs::read(self.state_dir().join("link.crt")).unwrap();
        pem.extend_from_slice(&std::fs::read(self.state_dir().join("link.key")).unwrap());
        let ca_pem = std::fs::read(&self.ca.ca_path).unwrap();
        reqwest::Client::builder()
            .use_rustls_tls()
            .add_root_certificate(reqwest::Certificate::from_pem(&ca_pem).unwrap())
            .identity(reqwest::Identity::from_pem(&pem).unwrap())
            .build()
            .unwrap()
    }

    /// The site's secret, from the state directory.
    pub fn secret(&self) -> String {
        let text = std::fs::read_to_string(self.state_dir().join("site.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json["secret"].as_str().unwrap().to_string()
    }

    pub fn link_url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.link_port, path)
    }

    /// Stop what runs and drop the database.
    pub async fn finish(mut self) {
        if let Some(link) = self.link.take() {
            let log = link.stop().await;
            assert!(!log.contains("panicked"), "the link panicked:\n{log}");
        }
        if let Some(admin) = self.admin.take() {
            let log = admin.stop().await;
            assert!(!log.contains("panicked"), "the admin panicked:\n{log}");
        }
        self.db.drop_db().await;
    }
}

pub async fn admin(
    port: u16,
    method: &str,
    path: &str,
    tenant: Option<&str>,
    body: Option<&str>,
) -> (u16, serde_json::Value) {
    let bearer = format!("Bearer {ADMIN_TOKEN}");
    let mut headers = vec![
        ("Authorization", bearer.as_str()),
        ("Content-Type", "application/json"),
    ];
    if let Some(tenant) = tenant {
        headers.push(("x-fcp-tenant-id", tenant));
    }
    let url = loopback(port, path);
    let (code, text) = http_request(method, &url, &headers, body)
        .await
        .unwrap_or_else(|e| panic!("{method} {url}: {e}"));
    (
        code,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
    )
}

/// Poll `path` until `check` holds.
pub async fn until<F: Fn(&serde_json::Value) -> bool>(
    what: &str,
    port: u16,
    path: &str,
    tenant: Option<&str>,
    check: F,
) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + A_WHILE;
    loop {
        let (code, body) = admin(port, "GET", path, tenant, None).await;
        if code == 200 && check(&body) {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: last {code} {body}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The cloud up with a tenant and a site, and the site's link enrolled
/// and up. `None` without a database.
pub async fn cloud_with_site() -> Option<Cloud> {
    cloud_with_site_at(None).await
}

/// As [`cloud_with_site`], the link given the site's database to keep
/// the tenant's configuration in.
pub async fn cloud_with_site_at(site_database_url: Option<&str>) -> Option<Cloud> {
    cloud_with_site_opts(CloudOptions {
        site_database_url,
        ..Default::default()
    })
    .await
}

/// What a cloud is started with.
#[derive(Default)]
pub struct CloudOptions<'a> {
    /// The site's database, for the link to keep the tenant's
    /// configuration in.
    pub site_database_url: Option<&'a str>,
    /// The cloud SBC's SIP-over-TLS address, `host:port`: set, every site
    /// enrolled gets a link trunk to it (`docs/HYBRID_CONTROL_PLANE.md`
    /// §3.6).
    pub sip_host: Option<String>,
    /// The cloud's events server ingest URL, where a site's events and the
    /// records the cloud writes for it are announced (§3.3).
    pub events_server_url: Option<String>,
    /// The site's Redis: its registrations for the heartbeat and its
    /// event stream for forwarding.
    pub site_redis_url: Option<String>,
    /// The directories the link may send files up from and remove files
    /// under.
    pub site_file_roots: Vec<String>,
    /// Put the link's connection to the cloud through a proxy the test can
    /// cut (`Cloud::link_cut`), as a network between a site and its cloud.
    pub cut_able: bool,
}

/// As [`cloud_with_site_at`], with everything a cloud may be started with.
pub async fn cloud_with_site_opts(opts: CloudOptions<'_>) -> Option<Cloud> {
    let site_database_url = opts.site_database_url;
    let sip_section = match &opts.sip_host {
        Some(host) => format!("sip_host = \"{host}\"\n"),
        None => String::new(),
    };
    let events_section = match &opts.events_server_url {
        Some(url) => format!("[events]\nserver_url = \"{url}\"\nauth_token = \"{EVENTS_TOKEN}\"\n"),
        None => String::new(),
    };
    let db = TestDatabase::create().await?;
    let dir = tempfile::tempdir().expect("tempdir");
    let certs_dir = dir.path().join("certs");
    std::fs::create_dir_all(&certs_dir).unwrap();
    let ca = ClusterCa::new(&certs_dir);
    let admin_cert = ca.node("fcp-admin");
    trust_ca(&ca.ca_path);
    let (http, enroll_port, link_port, link_health) =
        (free_port(), free_port(), free_port(), free_port());
    let link_cut = if opts.cut_able {
        Some(fcp_testkit::cut::CutLink::open(([127, 0, 0, 1], link_port).into()).await)
    } else {
        None
    };
    let link_reach_port = link_cut.as_ref().map(|c| c.port()).unwrap_or(link_port);
    let admin_config = format!(
        r#"prompts_dir = "{dir}/prompts"

[service]
node_id = "link-test-admin"

[api]
bind_address = "127.0.0.1:{http}"
auth_enabled = true
auth_token = "{ADMIN_TOKEN}"

[storage]
database_url = "{db}"

[link]
enabled = true
enroll_bind = "127.0.0.1:{enroll_port}"
bind = "127.0.0.1:{link_port}"
cert = "{cert}"
key = "{key}"
ca_dir = "{dir}/link-ca"
heartbeat_interval_secs = 1
heartbeat_stale_secs = 3
{sip_section}
{events_section}
[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(dir.path()),
        db = db.url(),
        cert = toml_path(&admin_cert.cert_path),
        key = toml_path(&admin_cert.key_path),
    );
    let admin_service = Service::start_with(
        "fcp-admin",
        bins::path("fcp-admin", "fcp-admin"),
        &admin_config,
        &loopback(http, "/api/v1/health"),
        &[],
    )
    .await;

    let (code, tenant) = admin(
        http,
        "POST",
        "/api/v1/tenants",
        None,
        Some(&serde_json::json!({ "name": "acme", "sip_domains": ["fcp.local"] }).to_string()),
    )
    .await;
    assert_eq!(code, 201, "{tenant}");
    let tenant_id = tenant["id"].as_str().unwrap().to_string();
    let (code, site) = admin(
        http,
        "POST",
        "/api/v1/sites",
        Some(&tenant_id),
        Some(&serde_json::json!({ "name": "acme-hq" }).to_string()),
    )
    .await;
    assert_eq!(code, 201, "{site}");
    let site_id = site["id"].as_str().unwrap().to_string();
    let token = site["enrollment_token"].as_str().unwrap().to_string();

    let mut site_section = String::from("[site]\nfeed_wait_secs = 5\nship_interval_secs = 1\n");
    if let Some(url) = site_database_url {
        site_section.push_str(&format!("database_url = \"{url}\"\n"));
    }
    if let Some(url) = &opts.site_redis_url {
        site_section.push_str(&format!("redis_url = \"{url}\"\nheartbeat_ttl_secs = 5\n"));
    }
    if !opts.site_file_roots.is_empty() {
        let roots: Vec<String> = opts
            .site_file_roots
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect();
        site_section.push_str(&format!("file_roots = [{}]\n", roots.join(", ")));
    }
    let link_config = format!(
        r#"[service]
node_id = "acme-hq-link"

[link]
cloud_url = "https://127.0.0.1:{enroll_port}"
link_url = "https://127.0.0.1:{link_reach_port}"
enrollment_token = "{token}"
state_dir = "{dir}/link-state"
cloud_ca = "{ca}"
heartbeat_interval_secs = 1
# Shorter than the feed's wait (5 s): an idle long poll must still not
# time out (docs/NIGHTLY_STACK.md §6).
request_timeout_secs = 3

{site_section}
[health]
bind_address = "127.0.0.1:{link_health}"

[logging]
level = "info"
format = "compact"
"#,
        dir = toml_path(dir.path()),
        ca = toml_path(&ca.ca_path),
    );
    let link = Service::start_with(
        "fcp-link",
        env!("CARGO_BIN_EXE_fcp-link"),
        &link_config,
        &loopback(link_health, "/health"),
        &[],
    )
    .await;
    let site_path = format!("/api/v1/sites/{site_id}");
    until(
        "the site never enrolled with its link up",
        http,
        &site_path,
        Some(&tenant_id),
        |s| s["status"] == "enrolled" && s["link_up"] == true,
    )
    .await;
    Some(Cloud {
        db,
        dir,
        ca,
        http,
        enroll_port,
        link_port,
        link_health,
        admin: Some(admin_service),
        link: Some(link),
        tenant_id,
        site_id,
        token,
        link_cut,
    })
}

/// What the stub events server saw: one entry per POST, with its headers.
#[derive(Debug, Clone)]
pub struct Posted {
    pub path: String,
    pub node_id: Option<String>,
    pub origin_site: Option<String>,
    pub body: serde_json::Value,
}

/// An events server that remembers what was posted to its ingest.
pub async fn stub_events_server() -> (u16, Arc<Mutex<Vec<Posted>>>) {
    use axum::{routing::post, Router};
    let seen: Arc<Mutex<Vec<Posted>>> = Arc::new(Mutex::new(Vec::new()));
    let port = free_port();
    let app = {
        let seen = Arc::clone(&seen);
        let handler = move |path: &'static str| {
            let seen = Arc::clone(&seen);
            move |headers: axum::http::HeaderMap, body: String| async move {
                let expected = format!("Bearer {EVENTS_TOKEN}");
                if headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .is_none_or(|v| v != expected)
                {
                    return axum::http::StatusCode::UNAUTHORIZED;
                }
                let header = |name: &str| {
                    headers
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                };
                seen.lock().unwrap().push(Posted {
                    path: path.to_string(),
                    node_id: header("x-node-id"),
                    origin_site: header("x-origin-site"),
                    body: serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
                });
                axum::http::StatusCode::ACCEPTED
            }
        };
        Router::new()
            .route("/ingest", post(handler("/ingest")))
            .route("/ingest/batch", post(handler("/ingest/batch")))
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, seen)
}

/// Poll `check` until it holds, or fail with `what`.
pub async fn eventually<F, Fut>(what: &str, within: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if check().await {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} did not happen within {within:?}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// A call manager's configuration for these tests: the cloud's or a
/// site's, on its own database, with what the test asks for.
#[derive(Default)]
pub struct CallManagerToml<'a> {
    pub node_id: &'a str,
    pub sip: u16,
    pub http: u16,
    pub db: &'a str,
    pub dir: &'a str,
    pub rtp_start: u16,
    /// The CA the node trusts for the TLS connections it opens (a site's
    /// link to its cloud).
    pub tls_ca: Option<&'a str>,
    /// The node's Redis, and its events published there.
    pub redis_url: Option<&'a str>,
}

impl CallManagerToml<'_> {
    pub fn render(&self) -> String {
        let tls_ca = match self.tls_ca {
            Some(path) => format!("tls_ca_path = \"{path}\"\n"),
            None => String::new(),
        };
        let redis = match self.redis_url {
            Some(url) => format!("redis_url = \"{url}\"\n"),
            None => String::new(),
        };
        let events = match self.redis_url {
            Some(_) => "[events]\nuse_redis = true\n".to_string(),
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
realm = "fcp.local"

[storage]
database_url = "{db}"
{redis}
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
# came from; the site's phones and the test's carriers are on the loopback.
[e911]
trusted_edges = ["127.0.0.0/8"]
{events}
[logging]
level = "info"
format = "compact"
"#,
            node_id = self.node_id,
            sip = self.sip,
            http = self.http,
            db = self.db,
            dir = self.dir,
            rtp_start = self.rtp_start,
            rtp_end = self.rtp_start + 60,
        )
    }
}
