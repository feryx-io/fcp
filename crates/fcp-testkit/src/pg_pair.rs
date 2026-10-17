//! A Postgres primary and its streaming standby, as processes the test
//! owns (`docs/DEEP_TESTING.md` D5c). The primary can be crashed and the
//! standby promoted, which is what Patroni or a managed database does on
//! a failover; with a [`CutLink`](crate::cut::CutLink) standing in for
//! the address the nodes are given (a VIP, a DNS name, a pooler),
//! retargeting the link is the address moving to the new primary.
//!
//! The server's binaries (`initdb`, `pg_ctl`, `pg_basebackup`) are found
//! in `FCP_PG_BIN` when set, else under `/usr/lib/postgresql/<major>/bin`
//! (the newest), else on the `PATH`. A test without them is skipped,
//! unless `FCP_PG_PAIR_TESTS=required` (as CI sets it), which makes the
//! absence a failure.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const DATABASE: &str = "fcp";

/// One server of the pair.
struct Server {
    data: PathBuf,
    port: u16,
}

pub struct PgPair {
    bin: PathBuf,
    dir: tempfile::TempDir,
    primary: Server,
    standby: Server,
}

fn find_bin() -> Option<PathBuf> {
    let has = |dir: &Path| dir.join("initdb").is_file() && dir.join("pg_basebackup").is_file();
    if let Ok(dir) = std::env::var("FCP_PG_BIN") {
        let dir = PathBuf::from(dir);
        return has(&dir).then_some(dir);
    }
    let mut majors: Vec<(u32, PathBuf)> = std::fs::read_dir("/usr/lib/postgresql")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let major = e.file_name().to_str()?.parse().ok()?;
            Some((major, e.path().join("bin")))
        })
        .filter(|(_, bin)| has(bin))
        .collect();
    majors.sort();
    if let Some((_, bin)) = majors.pop() {
        return Some(bin);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .find(|dir| has(dir))
}

fn run(bin: &Path, tool: &str, args: &[&str]) {
    let out = command(bin, tool)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{tool}: {e}"));
    assert!(
        out.status.success(),
        "{tool} {args:?}: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `tool` from `bin`. An install unpacked outside the system's library
/// path (a portable one) finds its own `lib`: the standby's WAL receiver
/// loads libpq from there.
fn command(bin: &Path, tool: &str) -> Command {
    let mut command = Command::new(bin.join(tool));
    if let Some(lib) = bin.parent().map(|p| p.join("lib")) {
        if lib.join("libpq.so.5").exists() {
            let mut paths = vec![lib];
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                paths.extend(std::env::split_paths(&old));
            }
            command.env("LD_LIBRARY_PATH", std::env::join_paths(paths).unwrap());
        }
    }
    command
}

fn url(port: u16) -> String {
    format!("postgresql://postgres@127.0.0.1:{port}/{DATABASE}")
}

impl PgPair {
    /// A primary with a database `fcp`, and a standby streaming from it,
    /// both running; `None` (skip) without the server's binaries.
    pub async fn start() -> Option<Self> {
        let required = std::env::var("FCP_PG_PAIR_TESTS").is_ok_and(|v| v == "required");
        let Some(bin) = find_bin() else {
            assert!(
                !required,
                "FCP_PG_PAIR_TESTS=required but the Postgres server's binaries are not \
                 installed (FCP_PG_BIN, /usr/lib/postgresql/<major>/bin or the PATH)"
            );
            return None;
        };
        let dir = tempfile::tempdir().unwrap();
        let primary = Server {
            data: dir.path().join("primary"),
            port: crate::free_port(),
        };
        let standby = Server {
            data: dir.path().join("standby"),
            port: crate::free_port(),
        };
        let pair = tokio::task::spawn_blocking(move || {
            let data = primary.data.to_str().unwrap();
            run(
                &bin,
                "initdb",
                &[
                    "-D",
                    data,
                    "-U",
                    "postgres",
                    "--auth=trust",
                    "-E",
                    "UTF8",
                    "--no-sync",
                ],
            );
            let pair = Self {
                bin,
                dir,
                primary,
                standby,
            };
            pair.configure(&pair.primary);
            // The standby replays the primary's WAL as it is written; the
            // replication slot keeps what it has not yet replayed.
            std::fs::OpenOptions::new()
                .append(true)
                .open(pair.primary.data.join("postgresql.conf"))
                .and_then(|mut f| {
                    std::io::Write::write_all(
                        &mut f,
                        b"wal_level = replica\nmax_wal_senders = 4\nmax_replication_slots = 4\n",
                    )
                })
                .unwrap();
            pair.pg_ctl(&pair.primary, "start");
            let port = pair.primary.port.to_string();
            run(
                &pair.bin,
                "createdb",
                &["-h", "127.0.0.1", "-p", &port, "-U", "postgres", DATABASE],
            );
            run(
                &pair.bin,
                "pg_basebackup",
                &[
                    "-h",
                    "127.0.0.1",
                    "-p",
                    &port,
                    "-U",
                    "postgres",
                    "-D",
                    pair.standby.data.to_str().unwrap(),
                    "-R",
                    "-X",
                    "stream",
                    "-C",
                    "-S",
                    "standby",
                    // At once, not over a spread checkpoint (minutes).
                    "-c",
                    "fast",
                    "--no-sync",
                ],
            );
            pair.configure(&pair.standby);
            pair.pg_ctl(&pair.standby, "start");
            pair
        })
        .await
        .unwrap();
        Some(pair)
    }

    /// Its own port, loopback only, its sockets in the pair's directory.
    fn configure(&self, server: &Server) {
        let conf = format!(
            "\nport = {}\nlisten_addresses = '127.0.0.1'\nunix_socket_directories = '{}'\n\
             fsync = off\nmax_connections = 200\nhot_standby = on\n",
            server.port,
            self.dir.path().display(),
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(server.data.join("postgresql.auto.conf"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, conf.as_bytes()))
            .unwrap();
    }

    fn pg_ctl(&self, server: &Server, action: &str) {
        let log = self.dir.path().join(format!("{}.log", server.port));
        let data = server.data.to_str().unwrap();
        let log = log.to_str().unwrap();
        match action {
            "start" => run(&self.bin, "pg_ctl", &["-D", data, "-l", log, "-w", "start"]),
            "crash" => run(
                &self.bin,
                "pg_ctl",
                &["-D", data, "-m", "immediate", "-w", "stop"],
            ),
            "promote" => run(&self.bin, "pg_ctl", &["-D", data, "-w", "promote"]),
            other => unreachable!("{other}"),
        }
    }

    /// The primary's database.
    pub fn primary_url(&self) -> String {
        url(self.primary.port)
    }

    /// The standby's database: read-only until promoted.
    pub fn standby_url(&self) -> String {
        url(self.standby.port)
    }

    /// Where the standby listens, for a link to move to.
    pub fn standby_addr(&self) -> std::net::SocketAddr {
        std::net::SocketAddr::from(([127, 0, 0, 1], self.standby.port))
    }

    /// Wait until the standby has replayed everything the primary has
    /// written, so a failover now loses nothing.
    pub async fn caught_up(&self, within: Duration) {
        let primary = sqlx::PgPool::connect(&self.primary_url()).await.unwrap();
        let standby = sqlx::PgPool::connect(&self.standby_url()).await.unwrap();
        let wanted: String = sqlx::query_scalar("SELECT pg_current_wal_lsn()::text")
            .fetch_one(&primary)
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let behind: bool = sqlx::query_scalar(
                "SELECT pg_last_wal_replay_lsn() IS NULL \
                 OR pg_last_wal_replay_lsn() < $1::pg_lsn",
            )
            .bind(&wanted)
            .fetch_one(&standby)
            .await
            .unwrap();
            if !behind {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the standby has not replayed {wanted} in {within:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        primary.close().await;
        standby.close().await;
    }

    /// The primary dies: stopped at once, its connections dropped, nothing
    /// flushed, as a crashed host.
    pub async fn crash_primary(&self) {
        self.blocking(|pair| pair.pg_ctl(&pair.primary, "crash"))
            .await;
    }

    /// The standby becomes the primary and takes writes.
    pub async fn promote_standby(&self) {
        self.blocking(|pair| pair.pg_ctl(&pair.standby, "promote"))
            .await;
    }

    /// A server's log, for a failure message.
    pub fn logs(&self) -> String {
        [&self.primary, &self.standby]
            .iter()
            .map(|s| {
                let path = self.dir.path().join(format!("{}.log", s.port));
                format!(
                    "── {} ──\n{}",
                    s.data.file_name().unwrap().to_string_lossy(),
                    std::fs::read_to_string(path).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn blocking(&self, f: impl FnOnce(&Self)) {
        tokio::task::block_in_place(|| f(self));
    }
}

impl Drop for PgPair {
    fn drop(&mut self) {
        for server in [&self.primary, &self.standby] {
            let _ = command(&self.bin, "pg_ctl")
                .args([
                    "-D",
                    server.data.to_str().unwrap(),
                    "-m",
                    "immediate",
                    "stop",
                ])
                .output();
        }
    }
}
