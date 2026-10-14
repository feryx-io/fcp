//! SIPp, a SIP implementation that is not ours (`docs/DEEP_TESTING.md`
//! D2). The testkit's [`Phone`](crate::Phone) is built on siphon-rs, the
//! stack every FCP binary runs, so a bug the two share passes every test
//! it drives. A scenario run by SIPp shares nothing with FCP.
//!
//! SIPp is found as `FCP_SIPP` when set, else `sipp` on the `PATH`. A test
//! without it is skipped, unless `FCP_SIPP_TESTS=required` (as CI sets
//! it), which makes the absence a failure.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// A SIPp binary.
#[derive(Clone, Debug)]
pub struct Sipp {
    bin: PathBuf,
}

/// One scenario run of SIPp, started and not yet waited for.
pub struct SippRun {
    name: String,
    child: tokio::process::Child,
    dir: tempfile::TempDir,
    /// The UDP port SIPp is bound to.
    pub port: u16,
}

/// How a run ended.
#[derive(Debug)]
pub struct SippOutcome {
    /// SIPp's exit code: 0 when every call passed, 1 when one failed.
    pub code: Option<i32>,
    /// Every message sent and received (`-trace_msg`).
    pub messages: String,
    /// SIPp's errors (`-trace_err`).
    pub errors: String,
}

impl SippOutcome {
    /// Whether SIPp sent a `code` response to a `method` request.
    pub fn sent_response(&self, code: u16, method: &str) -> bool {
        self.messages
            .split("-----------------------------------------------")
            .any(|block| {
                block.contains("message sent")
                    && block.contains(&format!("SIP/2.0 {code} "))
                    && block
                        .lines()
                        .any(|l| l.starts_with("CSeq:") && l.trim_end().ends_with(method))
            })
    }

    /// Whether SIPp received a `method` request.
    pub fn received_request(&self, method: &str) -> bool {
        self.messages
            .split("-----------------------------------------------")
            .any(|block| {
                block.contains("message received")
                    && block.lines().any(|l| l.starts_with(&format!("{method} ")))
            })
    }

    /// Panic with the run's errors and messages unless every call passed.
    pub fn assert_passed(&self, what: &str) {
        assert_eq!(
            self.code,
            Some(0),
            "{what}: SIPp failed\n--- errors ---\n{}\n--- messages ---\n{}",
            self.errors,
            self.messages
        );
    }
}

impl Sipp {
    /// SIPp, or `None` (the caller skips) when it is not installed and
    /// not required.
    pub fn find() -> Option<Self> {
        let required = std::env::var("FCP_SIPP_TESTS").is_ok_and(|v| v == "required");
        let bin = match std::env::var_os("FCP_SIPP") {
            Some(bin) => PathBuf::from(bin),
            None => std::env::var_os("PATH")
                .and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|dir| dir.join("sipp"))
                        .find(|p| p.is_file())
                })
                .unwrap_or_default(),
        };
        if bin.is_file() {
            return Some(Self { bin });
        }
        assert!(
            !required,
            "FCP_SIPP_TESTS=required but SIPp is not installed (FCP_SIPP or `sipp` on the PATH)"
        );
        eprintln!("SIPp not found (FCP_SIPP or `sipp` on the PATH); skipping");
        None
    }

    /// Start `scenario` (a SIPp XML file) against `target` over UDP on
    /// loopback: one call (`-m 1`) unless `args` say otherwise. `keys` are
    /// SIPp `-key` values, read in the scenario as `[name]`.
    pub fn start(
        &self,
        scenario: &Path,
        target: Option<SocketAddr>,
        keys: &[(&str, &str)],
        args: &[&str],
    ) -> SippRun {
        let name = scenario
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = tempfile::Builder::new()
            .prefix("fcp-sipp-")
            .tempdir()
            .expect("tempdir");
        let port = free_udp_port();
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("-sf")
            .arg(scenario)
            .args(["-i", "127.0.0.1", "-p", &port.to_string(), "-t", "u1"])
            .args(["-nostdin", "-trace_err", "-trace_msg"])
            .arg("-error_file")
            .arg(dir.path().join("errors.log"))
            .arg("-message_file")
            .arg(dir.path().join("messages.log"))
            .args(["-timeout", "30s", "-timeout_error"]);
        if !args.contains(&"-m") {
            cmd.args(["-m", "1"]);
        }
        for (key, value) in keys {
            cmd.args(["-key", key, value]);
        }
        cmd.args(args);
        if let Some(target) = target {
            cmd.arg(target.to_string());
        }
        // SIPp draws its statistics screen on stdout; nobody reads it.
        cmd.current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("start SIPp {}: {e}", self.bin.display()));
        SippRun {
            name,
            child,
            dir,
            port,
        }
    }

    /// [`Sipp::start`] and wait for it, for up to 40 seconds.
    pub async fn run(
        &self,
        scenario: &Path,
        target: SocketAddr,
        keys: &[(&str, &str)],
        args: &[&str],
    ) -> SippOutcome {
        self.start(scenario, Some(target), keys, args)
            .wait(Duration::from_secs(40))
            .await
    }
}

impl SippRun {
    /// This run's address, for a Contact that points at it.
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.port))
    }

    /// Wait for SIPp to exit, killing it after `within`.
    pub async fn wait(mut self, within: Duration) -> SippOutcome {
        let code = match tokio::time::timeout(within, self.child.wait()).await {
            Ok(status) => status.expect("wait for SIPp").code(),
            Err(_) => {
                let _ = self.child.kill().await;
                None
            }
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            use tokio::io::AsyncReadExt;
            let _ = pipe.read_to_string(&mut stderr).await;
        }
        let errors = format!(
            "{}{}",
            stderr,
            std::fs::read_to_string(self.dir.path().join("errors.log")).unwrap_or_default()
        );
        let messages =
            std::fs::read_to_string(self.dir.path().join("messages.log")).unwrap_or_default();
        if code.is_none() {
            eprintln!("SIPp {} did not finish in {within:?}; killed", self.name);
        }
        SippOutcome {
            code,
            messages,
            errors,
        }
    }
}

/// A UDP port free on loopback just now.
pub fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|s| s.local_addr())
        .expect("bind a UDP port")
        .port()
}
