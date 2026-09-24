//! Binary smoke tests: start a built service, wait for its health endpoint,
//! stop it with SIGTERM, and check that it exits cleanly.
//!
//! Each binary crate has a `tests/smoke.rs` that points [`Smoke`] at
//! `env!("CARGO_BIN_EXE_<name>")`, hands it a TOML config on free loopback
//! ports, and asserts on the resulting [`SmokeReport`]. Cargo builds the
//! binary before running the test, so `cargo test --all-targets` covers
//! every service without extra CI steps.

use std::io::Write;
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;

/// Pick a loopback port that is currently free for both TCP and UDP, so the
/// same number can serve a SIP `udp_bind`/`tcp_bind` pair.
///
/// The port is released before returning, so a concurrent process could in
/// principle grab it first. Smoke tests run one service per test binary, so
/// the window is small enough in practice.
pub fn free_port() -> u16 {
    // The OS may hand the same ephemeral port to two binds a moment apart
    // once the first is released, and a stack takes several ports before
    // any binary binds them (its SIP and HTTP ports once came out equal);
    // so a port is never given out twice by one test process.
    static GIVEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    for _ in 0..256 {
        let tcp = TcpListener::bind("127.0.0.1:0").expect("bind loopback tcp");
        let port = tcp.local_addr().unwrap().port();
        if UdpSocket::bind(("127.0.0.1", port)).is_err() {
            continue;
        }
        let mut given = GIVEN.lock().unwrap_or_else(|e| e.into_inner());
        if given.contains(&port) {
            continue;
        }
        given.push(port);
        return port;
    }
    panic!("no fresh port free for both TCP and UDP after 256 attempts");
}

/// Builder for one smoke run of a service binary.
pub struct Smoke {
    bin: PathBuf,
    args: Vec<String>,
    envs: Vec<(String, String)>,
    config: Option<String>,
    config_flag: String,
    health_url: Option<String>,
    startup_timeout: Duration,
    shutdown_timeout: Duration,
    #[allow(clippy::type_complexity)]
    while_up: Option<Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>>,
}

/// A probe run against the service while it is up.
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

impl Smoke {
    /// Target a binary; pass `env!("CARGO_BIN_EXE_<name>")`.
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            args: Vec::new(),
            envs: Vec::new(),
            config: None,
            config_flag: "--config".to_string(),
            health_url: None,
            startup_timeout: Duration::from_secs(45),
            shutdown_timeout: Duration::from_secs(20),
            while_up: None,
        }
    }

    /// Run this once the service is healthy and before it is stopped — the
    /// routes a test wants to exercise on a running binary, such as
    /// `/ready` and `/drain`. A panic inside it fails the test, and the
    /// service is killed as the child is dropped.
    pub fn while_up<F, Fut>(mut self, probe: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.while_up = Some(Box::new(move || Box::pin(probe())));
        self
    }

    /// TOML written to a temp file and passed via the config flag.
    pub fn config_toml(mut self, toml: impl Into<String>) -> Self {
        self.config = Some(toml.into());
        self
    }

    /// Flag that takes the config path (default `--config`).
    pub fn config_flag(mut self, flag: impl Into<String>) -> Self {
        self.config_flag = flag.into();
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// URL that must answer HTTP 200 before the service counts as started.
    pub fn health_url(mut self, url: impl Into<String>) -> Self {
        self.health_url = Some(url.into());
        self
    }

    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Start the service, wait for health, send SIGTERM, wait for exit.
    ///
    /// Never panics on the service's behaviour; inspect the report or call
    /// [`SmokeReport::assert_clean`].
    pub async fn run(self) -> SmokeReport {
        let health_url = self.health_url.clone().expect("health_url is required");
        let mut report = SmokeReport::default();

        // Keep the temp file alive for the whole run.
        let config_file = self.config.as_deref().map(write_temp_config);
        let mut command = Command::new(&self.bin);
        if let Some(file) = &config_file {
            command.arg(&self.config_flag).arg(file.path());
        }
        command
            .args(&self.args)
            .envs(self.envs.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                report.failure = Some(format!("failed to spawn {}: {e}", self.bin.display()));
                return report;
            }
        };
        let stdout = collect(child.stdout.take().expect("piped stdout"));
        let stderr = collect(child.stderr.take().expect("piped stderr"));

        // Phase 1: wait for health.
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                report.status = Some(status);
                report.failure = Some(format!(
                    "exited with {status} before {health_url} became healthy"
                ));
                break;
            }
            match http_get(&health_url).await {
                Ok((200, body)) => {
                    report.health_body = body;
                    report.startup = started.elapsed();
                    break;
                }
                Ok((code, body)) => report.last_health = Some(format!("HTTP {code}: {body}")),
                Err(e) => report.last_health = Some(e.to_string()),
            }
            if started.elapsed() > self.startup_timeout {
                report.failure = Some(format!(
                    "{health_url} not healthy after {:?} (last: {})",
                    self.startup_timeout,
                    report.last_health.as_deref().unwrap_or("no response")
                ));
                let _ = child.start_kill();
                let _ = child.wait().await;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        // Phase 1b: whatever the test wants to see on a running service.
        if report.failure.is_none() {
            if let Some(probe) = self.while_up {
                probe().await;
            }
        }

        // Phase 2: graceful stop.
        if report.failure.is_none() {
            let pid = child.id().expect("child still running") as libc::pid_t;
            // SAFETY: plain signal delivery to a child we spawned and still own.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            let stopping = Instant::now();
            match tokio::time::timeout(self.shutdown_timeout, child.wait()).await {
                Ok(Ok(status)) => {
                    report.status = Some(status);
                    report.shutdown = stopping.elapsed();
                    if !status.success() {
                        report.failure = Some(format!("exited with {status} after SIGTERM"));
                    }
                }
                Ok(Err(e)) => report.failure = Some(format!("wait failed: {e}")),
                Err(_) => {
                    report.failure = Some(format!(
                        "still running {:?} after SIGTERM; killed",
                        self.shutdown_timeout
                    ));
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
        }

        report.stdout = stdout.await;
        report.stderr = stderr.await;
        report
    }
}

/// Outcome of a [`Smoke::run`].
#[derive(Default, Debug)]
pub struct SmokeReport {
    /// Exit status, if the process exited.
    pub status: Option<ExitStatus>,
    /// Body of the first healthy response.
    pub health_body: String,
    /// Time from spawn to first healthy response.
    pub startup: Duration,
    /// Time from SIGTERM to exit.
    pub shutdown: Duration,
    /// Last failed health probe, for diagnostics.
    pub last_health: Option<String>,
    pub stdout: String,
    pub stderr: String,
    /// What went wrong, if anything.
    pub failure: Option<String>,
}

impl SmokeReport {
    /// Panic with captured output unless the service became healthy and then
    /// exited successfully on SIGTERM.
    pub fn assert_clean(&self) {
        if let Some(failure) = &self.failure {
            panic!(
                "smoke test failed: {failure}\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.stdout, self.stderr
            );
        }
    }

    /// Panic if `needle` does not appear in the combined stdout/stderr.
    pub fn assert_logged(&self, needle: &str) {
        if !self.stdout.contains(needle) && !self.stderr.contains(needle) {
            panic!(
                "expected {needle:?} in service output\n\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.stdout, self.stderr
            );
        }
    }
}

// ─── Blocking helpers for non-async `#[test]`s ────────────────────────────

/// `--version` must print the crate version; `--help` must exit 0 and
/// mention the config flag.
pub fn assert_version_and_help(bin: &str, expected_version: &str) {
    let out = run_blocking(bin, &["--version"], Duration::from_secs(10));
    assert!(
        out.status.success(),
        "--version exited {}\n{}",
        out.status,
        out.stderr
    );
    assert!(
        out.stdout.contains(expected_version),
        "--version printed {:?}, expected {expected_version}",
        out.stdout.trim()
    );

    let out = run_blocking(bin, &["--help"], Duration::from_secs(10));
    assert!(out.status.success(), "--help exited {}", out.status);
    assert!(
        out.stdout.contains("--config"),
        "--help does not mention --config:\n{}",
        out.stdout
    );
}

/// A syntactically invalid config file must make the binary exit non-zero
/// promptly instead of starting with defaults.
pub fn assert_rejects_malformed_config(bin: &str) {
    assert_rejects_malformed_config_with_flag(bin, "--config");
}

pub fn assert_rejects_malformed_config_with_flag(bin: &str, flag: &str) {
    let file = write_temp_config("this = is = not [valid toml\n");
    let path = file.path().to_string_lossy().into_owned();
    let out = run_blocking(bin, &[flag, &path], Duration::from_secs(15));
    assert!(
        !out.status.success(),
        "binary accepted a malformed config (exit {})\n{}",
        out.status,
        out.stdout
    );
    assert!(
        out.status.code().is_some(),
        "binary died from a signal on malformed config: {}",
        out.status
    );
}

/// Captured output of a short-lived process run.
pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Run a binary to completion, killing it if it exceeds `timeout`.
pub fn run_blocking(bin: &str, args: &[&str], timeout: Duration) -> Output {
    let mut child = std::process::Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_thread = std::thread::spawn(move || read_all(stdout));
    let err_thread = std::thread::spawn(move || read_all(stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let status = child.wait().expect("wait after kill");
            panic!("{bin} {args:?} did not exit within {timeout:?} (status {status})");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Output {
        status,
        stdout: out_thread.join().unwrap(),
        stderr: err_thread.join().unwrap(),
    }
}

fn read_all(mut reader: impl std::io::Read) -> String {
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

// ─── Internals ────────────────────────────────────────────────────────────

pub(crate) fn write_temp_config(contents: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new()
        .prefix("fcp-smoke-")
        .suffix(".toml")
        .tempfile()
        .expect("create temp config");
    file.write_all(contents.as_bytes())
        .expect("write temp config");
    file.flush().expect("flush temp config");
    file
}

/// Drain a child pipe into a string on a background task.
pub(crate) fn collect<R>(reader: R) -> impl std::future::Future<Output = String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let buffer = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&buffer);
    let task = tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut guard = sink.lock().unwrap();
            guard.push_str(&line);
            guard.push('\n');
        }
    });
    async move {
        let _ = task.await;
        let guard = buffer.lock().unwrap();
        guard.clone()
    }
}

/// Minimal HTTP/1.1 GET over a raw socket; returns `(status, body)`.
///
/// Avoids pulling an HTTP client into every binary's dev-dependencies.
pub async fn http_get(url: &str) -> anyhow::Result<(u16, String)> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("only http:// URLs are supported: {url}"))?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".to_string()),
    };

    let mut stream = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(authority))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout"))??;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;

    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw))
        .await
        .map_err(|_| anyhow::anyhow!("read timeout"))??;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("malformed response: {head:?}"))?;
    Ok((status, decode_body(head, body)))
}

/// The body as the server meant it: chunked transfer encoding undone
/// (a streaming body — anything behind a compression layer, say — is
/// sent in chunks even to a client that asked for no encoding).
fn decode_body(head: &str, body: &str) -> String {
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    if !chunked {
        return body.to_string();
    }
    let mut out = String::new();
    let mut rest = body;
    while let Some((size_line, after)) = rest.split_once("\r\n") {
        let size = size_line
            .split(';')
            .next()
            .and_then(|h| usize::from_str_radix(h.trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 || after.len() < size {
            break;
        }
        out.push_str(&after[..size]);
        rest = after[size..].strip_prefix("\r\n").unwrap_or(&after[size..]);
    }
    out
}

/// The repository root, from the testkit's own manifest directory.
pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

/// Every configuration a binary ships, run through `--check-config`
/// (`docs/SHIP_AND_OPERATE.md` phase 1). `must_start` are repo-relative
/// files the node must accept outright, exit 0; `must_parse` are files
/// that must at least parse (no unknown key, no wrong type: exit 0 or
/// exit 1), because what refuses them is a rule a later phase satisfies.
/// Every printed configuration must have its secrets masked. A listed
/// file that does not exist fails: the list is the claim.
pub fn assert_configs_check(
    bin: &str,
    extra_args: &[&str],
    must_start: &[&str],
    must_parse: &[&str],
) {
    let root = repo_root();
    let run = |rel: &str| {
        let path = root.join(rel);
        assert!(path.is_file(), "{rel} is listed but does not exist");
        let path = path.to_string_lossy().to_string();
        let mut args: Vec<&str> = vec!["--check-config", "--config", &path];
        args.extend_from_slice(extra_args);
        let out = run_blocking(bin, &args, Duration::from_secs(30));
        (out.status.code(), out.stdout.clone(), out.stderr.clone())
    };
    for rel in must_start {
        let (code, stdout, stderr) = run(rel);
        assert_eq!(code, Some(0), "{rel} must be accepted:\n{stderr}");
        assert_masked(rel, &stdout);
    }
    for rel in must_parse {
        let (code, stdout, stderr) = run(rel);
        assert!(
            code == Some(0) || code == Some(1),
            "{rel} must parse (exit 0 or 1, got {code:?}):\n{stderr}"
        );
        if code == Some(0) {
            assert_masked(rel, &stdout);
        }
    }
}

/// No printed line under a secret-looking key carries a value other
/// than the mask (or nothing).
fn assert_masked(rel: &str, stdout: &str) {
    let secret_key = |key: &str| {
        [
            "token",
            "secret",
            "password",
            "passwd",
            "jwt",
            "private_key",
            "master_key",
        ]
        .iter()
        .any(|n| key.contains(n))
            && !key.ends_with("_file")
            && !key.ends_with("_path")
            && !key.ends_with("_secs")
            && !key.ends_with("_enabled")
            && !key.ends_with("_required")
            && !key.starts_with("require_")
            && !key.starts_with("allow_")
            && !key.starts_with("disable_")
    };
    let leaks = |line: &str| {
        line.replace("\"***\"", "")
            .replace("\"\"", "")
            .contains('"')
    };
    // A multi-line array under a secret key is checked to its `]`.
    let mut in_secret_array = false;
    for line in stdout.lines() {
        if in_secret_array {
            assert!(!leaks(line), "{rel}: `{line}` is not masked");
            if line.trim_end().ends_with(']') {
                in_secret_array = false;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if !secret_key(&key) {
            continue;
        }
        let value = value.trim();
        assert!(!leaks(value), "{rel}: `{line}` is not masked");
        if value.starts_with('[') && !value.ends_with(']') {
            in_secret_array = true;
        }
    }
}

/// A secret given in the environment wins over the file
/// (`docs/SHIP_AND_OPERATE.md` phase 2): `bin --check-config` on
/// `config_toml` with `env` set must be accepted, its output must show
/// `must_show`, and the secret's value must appear nowhere in it.
pub fn assert_env_override(bin: &str, config_toml: &str, env: (&str, &str), must_show: &str) {
    let file = tempfile::Builder::new()
        .prefix("fcp-env-")
        .suffix(".toml")
        .tempfile()
        .expect("temp config");
    std::fs::write(file.path(), config_toml).expect("write config");
    let path = file.path().to_string_lossy().to_string();
    let out = std::process::Command::new(bin)
        .args(["--check-config", "--config", &path])
        .env(env.0, env.1)
        .output()
        .expect("run binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{bin} with {} set must be accepted:\n{stderr}",
        env.0
    );
    assert!(
        stdout.contains(must_show),
        "{bin}: `{must_show}` not shown with {} set:\n{stdout}",
        env.0
    );
    assert!(
        !stdout.contains(env.1),
        "{bin}: the secret from {} is printed:\n{stdout}",
        env.0
    );
}

/// Convenience for building a loopback URL.
pub fn loopback(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

/// Directory helper for tests that need writable paths in their config.
pub fn temp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("fcp-smoke-")
        .tempdir()
        .expect("create temp dir")
}

/// Render a filesystem path for embedding in TOML.
pub fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// One plain HTTP/1.1 request: the status and the body. `headers` are
/// extra request headers (`("Authorization", "Bearer …")`); a body is
/// sent as JSON.
pub async fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> anyhow::Result<(u16, String)> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("only http:// URLs are supported: {url}"))?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let mut stream = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(authority))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout"))??;
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n"
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    match body {
        Some(body) => request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )),
        None => request.push_str("\r\n"),
    }
    stream.write_all(request.as_bytes()).await?;
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut raw))
        .await
        .map_err(|_| anyhow::anyhow!("read timeout"))??;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("malformed response: {head:?}"))?;
    Ok((status, decode_body(head, body)))
}

/// `POST` a JSON body over plain HTTP/1.1: the status and the body.
pub async fn http_post(url: &str, body: &str) -> anyhow::Result<(u16, String)> {
    http_request("POST", url, &[], Some(body)).await
}

#[cfg(test)]
mod chunked_tests {
    use super::decode_body;

    #[test]
    fn a_chunked_body_is_reassembled() {
        let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked";
        let body = "5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n";
        assert_eq!(decode_body(head, body), "{\"a\":1}");
        assert_eq!(
            decode_body("HTTP/1.1 200 OK\r\ncontent-length: 2", "ok"),
            "ok"
        );
    }
}
