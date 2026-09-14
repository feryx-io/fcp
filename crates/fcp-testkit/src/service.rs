//! A service binary kept running for the length of a test: started on
//! the config it is given, healthy before it is handed back, stopped
//! with SIGTERM when the test is done (or killed when the test panics,
//! since the child is `kill_on_drop`). The smoke harness's start and
//! stop, without the one-shot shape.

use crate::smoke::{collect, http_get, write_temp_config};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

pub struct Service {
    name: String,
    child: Child,
    _config: Option<tempfile::NamedTempFile>,
    stdout: Option<std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>>,
    stderr: Option<std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>>>,
}

impl Service {
    /// Start `bin` with `--config <config_toml>` and wait until
    /// `health_url` answers 200 (up to 45 s). Panics, with the process's
    /// output, if it exits or never becomes healthy.
    pub async fn start(
        name: &str,
        bin: impl Into<PathBuf>,
        config_toml: &str,
        health_url: &str,
    ) -> Self {
        Self::start_with(name, bin, config_toml, health_url, &[]).await
    }

    /// As [`Service::start`], with environment variables.
    pub async fn start_with(
        name: &str,
        bin: impl Into<PathBuf>,
        config_toml: &str,
        health_url: &str,
        envs: &[(&str, &str)],
    ) -> Self {
        let bin = bin.into();
        let config = write_temp_config(config_toml);
        let mut command = Command::new(&bin);
        command
            .arg("--config")
            .arg(config.path())
            .envs(envs.iter().map(|(k, v)| (*k, *v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .unwrap_or_else(|e| panic!("{name}: failed to spawn {}: {e}", bin.display()));
        let stdout = collect(child.stdout.take().expect("piped stdout"));
        let stderr = collect(child.stderr.take().expect("piped stderr"));
        let mut service = Self {
            name: name.to_string(),
            child,
            _config: Some(config),
            stdout: Some(Box::pin(stdout)),
            stderr: Some(Box::pin(stderr)),
        };
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = service.child.try_wait() {
                let logs = service.drain_logs().await;
                panic!("{name}: exited with {status} before {health_url} became healthy\n{logs}");
            }
            let last = match http_get(health_url).await {
                Ok((200, _)) => break,
                Ok((code, body)) => format!("HTTP {code}: {body}"),
                Err(e) => e.to_string(),
            };
            if started.elapsed() > Duration::from_secs(45) {
                let _ = service.child.start_kill();
                let _ = service.child.wait().await;
                let logs = service.drain_logs().await;
                panic!("{name}: {health_url} not healthy after 45 s (last: {last})\n{logs}");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        service
    }

    /// SIGTERM, then wait up to 20 s. Returns the captured output; panics
    /// if the exit was not clean.
    pub async fn stop(mut self) -> String {
        let pid = self.child.id().expect("child still running") as libc::pid_t;
        // SAFETY: plain signal delivery to a child we spawned and still own.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        let outcome = tokio::time::timeout(Duration::from_secs(20), self.child.wait()).await;
        let logs = self.drain_logs().await;
        match outcome {
            Ok(Ok(status)) if status.success() => logs,
            Ok(Ok(status)) => panic!("{}: exited with {status} after SIGTERM\n{logs}", self.name),
            Ok(Err(e)) => panic!("{}: wait failed: {e}\n{logs}", self.name),
            Err(_) => {
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
                panic!(
                    "{}: still running 20 s after SIGTERM; killed\n{logs}",
                    self.name
                )
            }
        }
    }

    /// Kill outright (a crash is being simulated, or the test is done
    /// with a node it took down another way).
    pub async fn kill(mut self) -> String {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        self.drain_logs().await
    }

    async fn drain_logs(&mut self) -> String {
        let out = match self.stdout.take() {
            Some(f) => f.await,
            None => String::new(),
        };
        let err = match self.stderr.take() {
            Some(f) => f.await,
            None => String::new(),
        };
        format!(
            "--- {} stdout ---\n{out}\n--- {} stderr ---\n{err}",
            self.name, self.name
        )
    }
}
