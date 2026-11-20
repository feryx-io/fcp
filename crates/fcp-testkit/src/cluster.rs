//! A conference cluster for a test: two or more conference nodes on one
//! database and a Redis logical database of the test's own, trunked
//! under mutual TLS from a CA this kit mints (`docs/TEST_FLOOR.md`, T4
//! and T5). The call manager or WebRTC gateway a scenario puts in front
//! of the nodes is the scenario's own to start.

use crate::certs::ClusterCa;
use crate::media::allocate_rtp_range;
use crate::smoke::{http_get, loopback};
use crate::{bins, free_port, Service};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

/// The secret every node of a test cluster shares (`cluster.secret`).
pub const CLUSTER_SECRET: &str = "e2e-cluster-secret";

/// A Redis logical database (1–15) of this test's own, emptied before it
/// is handed out: a random base per process, then one per acquisition,
/// so membership, the registry and the streams never see another test's
/// nodes or records (`docs/NIGHTLY_STACK.md` §1.3). `None` without
/// `FCP_TEST_REDIS_URL`.
///
/// The flush is safe because cargo runs test binaries one at a time, so
/// whatever is in the database was left by an earlier binary; within a
/// binary the indexes are distinct, and a binary that asks for more than
/// the fifteen there are is refused rather than handed one in use.
pub fn private_redis_url() -> Option<String> {
    static BASE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let redis = std::env::var("FCP_TEST_REDIS_URL").ok()?;
    let base = *BASE.get_or_init(|| (uuid::Uuid::new_v4().as_u128() % 15) as u32);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    assert!(
        n < 15,
        "a test binary asked for a sixteenth private Redis database; there are fifteen"
    );
    Some(emptied(&redis, 1 + (base + n) % 15))
}

/// Database `index` of the Redis at `redis`, flushed; its URL.
fn emptied(redis: &str, index: u32) -> String {
    let url = format!("{}/{index}", redis.trim_end_matches('/'));
    let flushed = redis::Client::open(url.as_str())
        .and_then(|client| client.get_connection())
        .and_then(|mut conn| redis::cmd("FLUSHDB").query::<()>(&mut conn));
    if let Err(e) = flushed {
        panic!("the test's Redis database {index} could not be emptied: {e}");
    }
    url
}

/// The heartbeat record a node wrote under `cluster:node:<node_id>` in
/// the Redis at `redis_url`, as the JSON it wrote (`ClusterMember`), or
/// `None` while there is no record. What placement, migration and the
/// admin API read; a test asserts on what a node *says* here.
pub async fn heartbeat(redis_url: &str, node_id: &str) -> Option<serde_json::Value> {
    let client = redis::Client::open(redis_url).ok()?;
    let mut conn = client.get_multiplexed_async_connection().await.ok()?;
    let json: Option<String> = redis::cmd("GET")
        .arg(format!("fcp:cluster:node:{node_id}"))
        .query_async(&mut conn)
        .await
        .ok()?;
    serde_json::from_str(&json?).ok()
}

/// `heartbeat` polled until `accept` says yes or `within` passes; the
/// last record seen either way (`None` if there never was one).
pub async fn heartbeat_when(
    redis_url: &str,
    node_id: &str,
    within: Duration,
    accept: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    let deadline = tokio::time::Instant::now() + within;
    let mut last = None;
    loop {
        if let Some(record) = heartbeat(redis_url, node_id).await {
            if accept(&record) {
                return Some(record);
            }
            last = Some(record);
        }
        if tokio::time::Instant::now() >= deadline {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// One conference node.
pub struct Node {
    pub id: String,
    pub service: Option<Service>,
    /// Where a phone dials a room on this node.
    pub sip: SocketAddr,
    pub http: u16,
}

impl Node {
    /// `GET /conferences/{room}` here, when the room is here.
    pub async fn room(&self, room: &str) -> Option<serde_json::Value> {
        let (code, body) = http_get(&loopback(self.http, &format!("/conferences/{room}")))
            .await
            .ok()?;
        (code == 200)
            .then(|| serde_json::from_str(&body).ok())
            .flatten()
    }

    /// The room's participant ids here (none when the room is not here).
    pub async fn participants(&self, room: &str) -> Vec<String> {
        self.room(room)
            .await
            .and_then(|r| {
                r["participants"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    /// One participant's detail here.
    pub async fn participant(&self, room: &str, pid: &str) -> serde_json::Value {
        let (code, body) = http_get(&loopback(
            self.http,
            &format!("/conferences/{room}/participants/{pid}"),
        ))
        .await
        .unwrap_or_else(|e| panic!("participant {pid}: {e}"));
        assert_eq!(code, 200, "participant {pid} on {}: {body}", self.id);
        serde_json::from_str(&body).unwrap()
    }

    /// The id of the participant here whose caller id or display name
    /// contains `name`.
    pub async fn find(&self, room: &str, name: &str) -> Option<String> {
        for pid in self.participants(room).await {
            let p = self.participant(room, &pid).await;
            let caller = p["caller_id"].as_str().unwrap_or("");
            let display = p["display_name"].as_str().unwrap_or("");
            if caller.contains(name) || display.contains(name) {
                return Some(pid);
            }
        }
        None
    }

    /// SIGTERM the node: it drains, and the logs come back.
    pub async fn stop(&mut self) -> String {
        match self.service.take() {
            Some(s) => s.stop().await,
            None => String::new(),
        }
    }
}

/// What a cluster is started with.
pub struct ClusterOptions {
    pub nodes: usize,
    /// `conference.join_secret` on every node, for a browser's join token.
    pub join_secret: Option<String>,
    /// More `[conference]` lines on every node, as written.
    pub conference_extra: String,
    /// More `[sip]` lines on every node, as written.
    pub sip_extra: String,
    /// RTP ports each node takes (120): a load run needs thousands.
    pub rtp_ports: u16,
    /// The first node's RTP ports instead, when it needs fewer: a
    /// webinar's stage with its audience on the other nodes.
    pub first_node_rtp_ports: Option<u16>,
    /// The nodes' log level (`info`).
    pub log_level: String,
}

impl Default for ClusterOptions {
    fn default() -> Self {
        Self {
            nodes: 2,
            join_secret: None,
            conference_extra: String::new(),
            sip_extra: String::new(),
            rtp_ports: 120,
            first_node_rtp_ports: None,
            log_level: "info".to_string(),
        }
    }
}

/// The nodes, up and heartbeating.
pub struct ConferenceCluster {
    pub nodes: Vec<Node>,
    pub redis_url: String,
    _ca: ClusterCa,
}

impl ConferenceCluster {
    /// Start the nodes on `database_url`, writing certificates under
    /// `dir`. `None` without `FCP_TEST_REDIS_URL`.
    pub async fn start(database_url: &str, dir: &Path, opts: ClusterOptions) -> Option<Self> {
        let redis_url = private_redis_url()?;
        let ca = ClusterCa::new(dir);
        let suffix = uuid::Uuid::new_v4().simple().to_string()[..6].to_string();
        let bin = bins::path("fcp-conference", "fcp-conference-server");
        let join_secret = opts
            .join_secret
            .as_deref()
            .map(|s| format!("join_secret = \"{s}\"\n"))
            .unwrap_or_default();
        let mut nodes = Vec::new();
        for i in 0..opts.nodes {
            let name = (b'a' + i as u8) as char;
            let id = format!("conf-{name}-{suffix}");
            let cert = ca.node(&id);
            let sip = free_port();
            let tls = free_port();
            let http = free_port();
            let rtp_ports = match opts.first_node_rtp_ports {
                Some(ports) if i == 0 => ports,
                _ => opts.rtp_ports,
            };
            let rtp_start = allocate_rtp_range(rtp_ports);
            let config = format!(
                r#"
[service]
node_id = "{id}"
drain_wait_secs = 0

[sip]
udp_bind = "127.0.0.1:{sip}"
tcp_bind = "127.0.0.1:{sip}"
tls_bind = "127.0.0.1:{tls}"
tls_cert_path = "{cert}"
tls_key_path = "{key}"
tls_client_ca_path = "{ca}"
local_ip = "127.0.0.1"
local_uri = "sip:conference@127.0.0.1:{sip}"
# The call manager and the gateway beside these nodes name the record
# each leg belongs to (`docs/EDGE_SECURITY.md` §2.3), so a room here
# believes what they say. Inter-node trunks need no listing: they prove
# themselves with a client certificate.
trusted_edges = ["127.0.0.1"]
{sip_extra}

[storage]
database_url = "{db}"
redis_url = "{redis}"

[http]
bind_address = "127.0.0.1:{http}"

[media]
rtp_port_start = {rtp_start}
rtp_port_end = {rtp_end}

[conference]
recordings_dir = "{dir}/recordings-{name}"
announcements_dir = "{dir}/announcements"
{join_secret}{extra}
[cluster]
enabled = true
secret = "{secret}"
heartbeat_interval_secs = 1
trunk_reconcile_interval_secs = 1
migration_timeout_secs = 10

[logging]
level = "{log_level}"
format = "compact"
"#,
                sip_extra = opts.sip_extra,
                log_level = opts.log_level,
                cert = cert.cert_path.display(),
                key = cert.key_path.display(),
                ca = ca.ca_path.display(),
                db = database_url,
                redis = redis_url,
                rtp_end = rtp_start + rtp_ports,
                dir = dir.display(),
                secret = CLUSTER_SECRET,
                extra = opts.conference_extra,
            );
            let service = Service::start(&id, &bin, &config, &loopback(http, "/health")).await;
            nodes.push(Node {
                id,
                service: Some(service),
                sip: SocketAddr::from(([127, 0, 0, 1], sip)),
                http,
            });
        }
        Some(Self {
            nodes,
            redis_url,
            _ca: ca,
        })
    }

    /// The node that lists a participant of `room` whose caller id or
    /// display name contains `name`, and the participant id; waits up
    /// to `within` for the room to settle.
    pub async fn where_is(
        &self,
        room: &str,
        name: &str,
        within: Duration,
    ) -> Option<(usize, String)> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            for (i, node) in self.nodes.iter().enumerate() {
                if node.service.is_none() {
                    continue;
                }
                if let Some(pid) = node.find(room, name).await {
                    return Some((i, pid));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Kill every node still running; their logs, for a failure.
    pub async fn kill_all(&mut self) -> String {
        let mut logs = String::new();
        for node in &mut self.nodes {
            if let Some(s) = node.service.take() {
                logs.push_str(&s.kill().await);
            }
        }
        logs
    }

    /// Stop every node still running (a clean exit asserted).
    pub async fn stop_all(&mut self) {
        for node in &mut self.nodes {
            if let Some(s) = node.service.take() {
                s.stop().await;
            }
        }
    }
}

/// Every event in the event streams of the Redis database at
/// `redis_url`, oldest first, as each envelope's `event` (what a node
/// published, `fcp_storage_redis::event_bus`). What a fault test reads to
/// see that events raised while Redis was away arrived once it was back.
pub async fn streamed_events(redis_url: &str) -> Vec<serde_json::Value> {
    let client = redis::Client::open(redis_url).expect("a Redis URL");
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .expect("Redis answers");
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("*event_stream*")
        .query_async(&mut conn)
        .await
        .expect("KEYS");
    let mut events = Vec::new();
    for key in keys {
        let reply: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
            .arg(&key)
            .arg("-")
            .arg("+")
            .query_async(&mut conn)
            .await
            .expect("XRANGE");
        for entry in reply.ids {
            let data: Option<String> = entry.get("data");
            if let Some(envelope) =
                data.and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok())
            {
                events.push(envelope["event"].clone());
            }
        }
    }
    events
}

#[cfg(test)]
mod private_redis_tests {
    use super::*;

    /// Two acquisitions are two databases, and each is empty when handed
    /// out, whatever an earlier binary left there.
    #[test]
    fn a_tests_redis_is_its_own_and_empty() {
        let Ok(redis) = std::env::var("FCP_TEST_REDIS_URL") else {
            eprintln!("skipped: FCP_TEST_REDIS_URL is not set");
            return;
        };
        let first = private_redis_url().unwrap();
        let second = private_redis_url().unwrap();
        assert_ne!(first, second, "two acquisitions, two databases");

        // What an earlier binary left in a database...
        let index: u32 = first.rsplit('/').next().unwrap().parse().unwrap();
        let mut conn = redis::Client::open(first.as_str())
            .unwrap()
            .get_connection()
            .unwrap();
        let _: () = redis::cmd("SET")
            .arg("fcp:cdrs:left-behind")
            .arg("1")
            .query(&mut conn)
            .unwrap();
        // ...is gone when the database is handed out again.
        emptied(&redis, index);
        let size: u64 = redis::cmd("DBSIZE").query(&mut conn).unwrap();
        assert_eq!(size, 0);
    }
}
