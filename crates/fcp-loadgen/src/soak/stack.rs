//! What the whole stack holds while a soak runs (`docs/DEEP_TESTING.md`
//! D6): each service's resident memory, open file descriptors, UDP
//! sockets (the RTP ports a media path holds) and threads; the keys and
//! memory in Redis; Postgres's connections and every table's live rows.
//!
//! A service is either a container of the compose project — read from
//! inside it with `docker exec`, since the binaries run as root there
//! and the host's `/proc/<pid>/fd` is not this user's to read — or a
//! process of this user's, read from `/proc` directly. Both are parsed
//! by the same functions. A probe that fails leaves its series out of
//! that sample rather than ending the run.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::time::Duration;
use tracing::debug;

/// One service the probe reads.
#[derive(Debug, Clone)]
pub enum Process {
    /// A compose service's container; the binary is its PID 1.
    Container { service: String, container: String },
    /// A process of this user's.
    Local { service: String, pid: u32 },
}

impl Process {
    pub fn service(&self) -> &str {
        match self {
            Self::Container { service, .. } | Self::Local { service, .. } => service,
        }
    }
}

/// One moment of the stack: series name → value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackSample {
    /// Seconds since the soak began.
    pub at_secs: f64,
    pub series: BTreeMap<String, f64>,
}

/// Reads the stack.
pub struct StackProbe {
    processes: Vec<Process>,
    postgres: Option<sqlx::PgPool>,
    redis: Option<(redis::aio::MultiplexedConnection, i64)>,
    /// Gauges read from a service's `/metrics` for context — the calls up
    /// while the sample was taken — under `metric/<name>`.
    metrics: Vec<MetricsSource>,
}

/// A service's Prometheus endpoint, and the bearer token it wants.
#[derive(Debug, Clone)]
pub struct MetricsSource {
    pub service: String,
    pub url: String,
    pub token: Option<String>,
}

impl StackProbe {
    /// Connect to the stores. A store that cannot be reached is left out
    /// of every sample, and said so once.
    pub async fn new(
        processes: Vec<Process>,
        database_url: Option<&str>,
        redis_url: Option<&str>,
        metrics: Vec<MetricsSource>,
    ) -> Self {
        let postgres = match database_url {
            Some(url) => match sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(10))
                .connect(url)
                .await
            {
                Ok(pool) => Some(pool),
                Err(e) => {
                    tracing::warn!(error = %e, "Postgres is not sampled");
                    None
                }
            },
            None => None,
        };
        let redis = match redis_url {
            Some(url) => {
                let db = redis_db(url);
                match redis::Client::open(url) {
                    Ok(client) => match client.get_multiplexed_async_connection().await {
                        Ok(conn) => Some((conn, db)),
                        Err(e) => {
                            tracing::warn!(error = %e, "Redis is not sampled");
                            None
                        }
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, "Redis is not sampled");
                        None
                    }
                }
            }
            None => None,
        };
        Self {
            processes,
            postgres,
            redis,
            metrics,
        }
    }

    pub fn processes(&self) -> &[Process] {
        &self.processes
    }

    /// Everything, once.
    pub async fn sample(&mut self, at_secs: f64) -> StackSample {
        let mut series = BTreeMap::new();
        for process in &self.processes {
            let read = match process {
                Process::Container { container, .. } => read_container(container).await,
                Process::Local { pid, .. } => read_local(*pid),
            };
            match read {
                Some(stats) => {
                    let s = process.service();
                    series.insert(format!("process/{s}/rss_bytes"), stats.rss_bytes as f64);
                    series.insert(format!("process/{s}/fds"), stats.fds as f64);
                    series.insert(format!("process/{s}/udp_sockets"), stats.udp_sockets as f64);
                    series.insert(format!("process/{s}/threads"), stats.threads as f64);
                }
                None => debug!(service = process.service(), "could not read the process"),
            }
        }
        if let Some(pool) = &self.postgres {
            if let Err(e) = sample_postgres(pool, &mut series).await {
                debug!(error = %e, "Postgres sample failed");
            }
        }
        if let Some((conn, db)) = &mut self.redis {
            if let Err(e) = sample_redis(conn, *db, &mut series).await {
                debug!(error = %e, "Redis sample failed");
            }
        }
        for source in &self.metrics {
            let bearer = source.token.as_ref().map(|t| format!("Bearer {t}"));
            let headers: Vec<(&str, &str)> = bearer
                .as_deref()
                .map(|b| vec![("Authorization", b)])
                .unwrap_or_default();
            let scraped = tokio::time::timeout(
                Duration::from_secs(10),
                fcp_testkit::smoke::http_request("GET", &source.url, &headers, None),
            )
            .await;
            if let Ok(Ok((200, body))) = scraped {
                for (metric, value) in crate::sampler::parse_prometheus(&body) {
                    // Gauges only: a counter grows by design.
                    if !metric.ends_with("_total") && !metric.ends_with("_count") {
                        series.insert(format!("metric/{}/{metric}", source.service), value);
                    }
                }
            }
        }
        StackSample { at_secs, series }
    }
}

/// What one process holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessStats {
    pub rss_bytes: u64,
    pub fds: u64,
    pub udp_sockets: u64,
    pub threads: u64,
}

/// The parts of a process's `/proc` the probe reads, as text.
struct ProcText {
    statm: String,
    status: String,
    /// One `readlink` target per open descriptor.
    fd_links: Vec<String>,
    /// `/proc/net/udp` and `/proc/net/udp6` of the process's namespace.
    udp: String,
}

fn stats_from(text: &ProcText) -> Option<ProcessStats> {
    let pages: u64 = text.statm.split_whitespace().nth(1)?.parse().ok()?;
    let threads = text
        .status
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let udp_inodes = udp_inodes(&text.udp);
    let udp_sockets = text
        .fd_links
        .iter()
        .filter_map(|l| socket_inode(l))
        .filter(|inode| udp_inodes.contains(inode))
        .count() as u64;
    Some(ProcessStats {
        rss_bytes: pages * 4096,
        fds: text.fd_links.len() as u64,
        udp_sockets,
        threads,
    })
}

/// `socket:[12345]` → 12345.
fn socket_inode(link: &str) -> Option<u64> {
    link.trim()
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// The inodes of `/proc/net/udp{,6}`: the tenth field of every row.
fn udp_inodes(table: &str) -> HashSet<u64> {
    table
        .lines()
        .filter(|l| !l.trim_start().starts_with("sl"))
        .filter_map(|l| l.split_whitespace().nth(9)?.parse().ok())
        .filter(|inode| *inode != 0)
        .collect()
}

fn read_local(pid: u32) -> Option<ProcessStats> {
    let proc_dir = format!("/proc/{pid}");
    let fd_links = std::fs::read_dir(format!("{proc_dir}/fd"))
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let udp = ["udp", "udp6"]
        .iter()
        .filter_map(|t| std::fs::read_to_string(format!("{proc_dir}/net/{t}")).ok())
        .collect::<Vec<_>>()
        .join("\n");
    stats_from(&ProcText {
        statm: std::fs::read_to_string(format!("{proc_dir}/statm")).ok()?,
        status: std::fs::read_to_string(format!("{proc_dir}/status")).ok()?,
        fd_links,
        udp,
    })
}

/// The separator between the parts `docker exec` prints.
const PART: &str = "@@fcp-soak@@";

async fn read_container(container: &str) -> Option<ProcessStats> {
    // Debian's `sh` and coreutils are in every FCP image.
    let script = format!(
        "cat /proc/1/statm; echo {PART}; cat /proc/1/status; echo {PART}; \
         for f in /proc/1/fd/*; do readlink \"$f\"; done; echo {PART}; \
         cat /proc/net/udp /proc/net/udp6 2>/dev/null"
    );
    let out = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new("docker")
            .args(["exec", container, "sh", "-c", &script])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_container_output(&String::from_utf8_lossy(&out.stdout))
}

fn parse_container_output(out: &str) -> Option<ProcessStats> {
    let parts: Vec<&str> = out.split(PART).collect();
    if parts.len() != 4 {
        return None;
    }
    stats_from(&ProcText {
        statm: parts[0].trim().to_string(),
        status: parts[1].to_string(),
        fd_links: parts[2]
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
        udp: parts[3].to_string(),
    })
}

/// The compose project's running FCP services as containers: every
/// service whose name starts `fcp-` but the nginx proxy.
pub async fn compose_processes(compose_dir: &std::path::Path) -> anyhow::Result<Vec<Process>> {
    let out = tokio::process::Command::new("docker")
        .current_dir(compose_dir)
        .args(["compose", "ps", "--format", "{{.Service}} {{.Name}}"])
        .output()
        .await?;
    anyhow::ensure!(
        out.status.success(),
        "docker compose ps: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(parse_compose_ps(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_compose_ps(out: &str) -> Vec<Process> {
    out.lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(service, _)| service.starts_with("fcp-") && *service != "fcp-proxy")
        .map(|(service, name)| Process::Container {
            service: service.to_string(),
            container: name.trim().to_string(),
        })
        .collect()
}

async fn sample_postgres(
    pool: &sqlx::PgPool,
    series: &mut BTreeMap<String, f64>,
) -> Result<(), sqlx::Error> {
    // Every connection to the database but this probe's own.
    let (connections,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND pid <> pg_backend_pid()",
    )
    .fetch_one(pool)
    .await?;
    series.insert("postgres/connections".into(), connections as f64);
    // Live rows by the statistics the server keeps, which every insert
    // and delete updates at commit: exact enough, and no table scans.
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT relname::text, n_live_tup FROM pg_stat_user_tables WHERE schemaname = 'public'",
    )
    .fetch_all(pool)
    .await?;
    for (table, live) in rows {
        series.insert(format!("table/{table}/rows"), live as f64);
    }
    Ok(())
}

async fn sample_redis(
    conn: &mut redis::aio::MultiplexedConnection,
    db: i64,
    series: &mut BTreeMap<String, f64>,
) -> redis::RedisResult<()> {
    let info: String = redis::cmd("INFO").query_async(conn).await?;
    let fields = info_fields(&info);
    series.insert("redis/keys".into(), keyspace_keys(&fields, db) as f64);
    for (field, name) in [
        ("used_memory", "redis/used_memory"),
        ("connected_clients", "redis/clients"),
    ] {
        if let Some(v) = fields.get(field).and_then(|v| v.parse::<f64>().ok()) {
            series.insert(name.into(), v);
        }
    }
    Ok(())
}

fn info_fields(info: &str) -> BTreeMap<String, String> {
    info.lines()
        .filter_map(|l| l.trim().split_once(':'))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `db3:keys=12,expires=4,avg_ttl=0` → 12 for database 3.
fn keyspace_keys(fields: &BTreeMap<String, String>, db: i64) -> u64 {
    fields
        .get(&format!("db{db}"))
        .and_then(|v| {
            v.split(',')
                .find_map(|kv| kv.strip_prefix("keys="))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0)
}

/// The database a Redis URL selects: its path, else 0.
fn redis_db(url: &str) -> i64 {
    url.rsplit_once('/')
        .and_then(|(before, db)| (!before.ends_with('/')).then_some(db))
        .and_then(|db| db.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UDP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
  12: 0100007F:13C4 00000000:0000 07 00000000:00000000 00:00000000 00000000     0        0 4242 2 0000000000000000 0
  13: 0100007F:4E20 00000000:0000 07 00000000:00000000 00:00000000 00000000     0        0 4343 2 0000000000000000 0
";

    #[test]
    fn a_process_is_read_from_the_text_of_its_proc() {
        let text = ProcText {
            statm: "50000 2500 300 10 0 900 0".into(),
            status: "Name:\tfcp\nThreads:\t17\n".into(),
            fd_links: vec![
                "/dev/null".into(),
                "socket:[4242]".into(),
                "socket:[4343]".into(),
                "socket:[9999]".into(),
                "anon_inode:[eventpoll]".into(),
            ],
            udp: UDP.into(),
        };
        let stats = stats_from(&text).unwrap();
        assert_eq!(
            stats,
            ProcessStats {
                rss_bytes: 2500 * 4096,
                fds: 5,
                udp_sockets: 2,
                threads: 17
            },
            "a TCP socket (9999) is a descriptor but not a UDP socket"
        );
    }

    #[test]
    fn a_containers_output_is_split_into_its_parts() {
        let out = format!(
            "100 20 0 0 0 0 0\n{PART}\nThreads:\t3\n{PART}\nsocket:[4242]\n/dev/null\n{PART}\n{UDP}"
        );
        let stats = parse_container_output(&out).unwrap();
        assert_eq!(stats.fds, 2);
        assert_eq!(stats.udp_sockets, 1);
        assert_eq!(stats.threads, 3);
        assert!(parse_container_output("Error: no such container").is_none());
    }

    #[test]
    fn this_process_reads_like_a_service() {
        let _socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let stats = read_local(std::process::id()).unwrap();
        assert!(stats.rss_bytes > 0 && stats.threads > 0);
        assert!(stats.udp_sockets >= 1, "{stats:?}");
    }

    #[test]
    fn compose_ps_names_the_fcp_services_but_the_proxy() {
        let ps = "postgres fcp-postgres\nfcp-admin docker-compose-fcp-admin-1\n\
                  fcp-proxy docker-compose-fcp-proxy-1\nfcp-call-manager fcp-call-manager\n";
        let names: Vec<String> = parse_compose_ps(ps)
            .iter()
            .map(|p| p.service().to_string())
            .collect();
        assert_eq!(names, ["fcp-admin", "fcp-call-manager"]);
    }

    #[test]
    fn redis_keys_are_those_of_the_database_the_url_selects() {
        let fields = info_fields(
            "# Keyspace\r\ndb0:keys=5,expires=1,avg_ttl=0\r\ndb7:keys=12,expires=0,avg_ttl=0\r\n",
        );
        assert_eq!(keyspace_keys(&fields, 7), 12);
        assert_eq!(keyspace_keys(&fields, 0), 5);
        assert_eq!(keyspace_keys(&fields, 3), 0);
        assert_eq!(redis_db("redis://127.0.0.1:6379/7"), 7);
        assert_eq!(redis_db("redis://127.0.0.1:6379"), 0);
        assert_eq!(redis_db("redis://redis:6379/"), 0);
    }
}
