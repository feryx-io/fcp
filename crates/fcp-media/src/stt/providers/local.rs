//! faster-whisper in a long-lived Python worker, like the kokoro TTS
//! worker: `scripts/whisper_stt_worker.py` loads the model once and reads
//! requests from stdin.
//!
//! ```text
//! stdout: {"ready": true, "duration_ms": 4200}              (once, at start)
//! stdin:  {"wav": "/tmp/x.wav", "language": "en", "prompt": "sales, support"}
//! stdout: {"ok": true, "text": "sales please", "confidence": 0.82}
//! ```
//!
//! The model is `base` unless configured (`WHISPER_MODEL`), on the device
//! configured (`WHISPER_DEVICE`, `auto` by default). Whisper has no
//! stream, so a call's audio is cut into utterances and each sent whole.

use super::super::{primary_language, wav_bytes, SttOptions, SttProvider, Transcript};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Recognition by a local whisper worker.
pub struct LocalStt {
    python: PathBuf,
    script: PathBuf,
    model: Option<String>,
    device: Option<String>,
    worker: Mutex<Option<Worker>>,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Serialize)]
struct Request<'a> {
    wav: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    ready: Option<bool>,
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
}

impl LocalStt {
    /// The worker is found at `FCP_WHISPER_WORKER`, in the checkout the
    /// executable was built in (`target/<profile>/../../scripts`), or at
    /// `scripts/whisper_stt_worker.py`, as the kokoro worker is.
    pub fn new(python: Option<PathBuf>, model: Option<String>, device: Option<String>) -> Self {
        let script = std::env::var("FCP_WHISPER_WORKER")
            .map(PathBuf::from)
            .ok()
            .or_else(|| {
                let checkout = std::env::current_exe()
                    .ok()?
                    .parent()?
                    .parent()?
                    .parent()?
                    .join("scripts/whisper_stt_worker.py");
                checkout.exists().then_some(checkout)
            })
            .unwrap_or_else(|| PathBuf::from("scripts/whisper_stt_worker.py"));
        Self {
            python: python.unwrap_or_else(|| PathBuf::from("python3")),
            script,
            model,
            device,
            worker: Mutex::new(None),
        }
    }

    async fn start(&self) -> Result<Worker> {
        info!(
            script = %self.script.display(),
            python = %self.python.display(),
            model = ?self.model,
            "starting the whisper worker"
        );
        let mut command = Command::new(&self.python);
        command
            .arg(&self.script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        if let Some(model) = &self.model {
            command.env("WHISPER_MODEL", model);
        }
        if let Some(device) = &self.device {
            command.env("WHISPER_DEVICE", device);
        }
        let mut child = command.spawn().with_context(|| {
            format!(
                "starting {} {}",
                self.python.display(),
                self.script.display()
            )
        })?;
        let stdin = child.stdin.take().context("the worker's stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("the worker's stdout")?);
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(300),
            stdout.read_line(&mut line),
        )
        .await
        .context("the whisper worker did not start in five minutes (a model download?)")??;
        let ready: Response = serde_json::from_str(line.trim())
            .with_context(|| format!("the whisper worker's first line: {}", line.trim()))?;
        if ready.ready != Some(true) {
            bail!("the whisper worker did not start: {}", line.trim());
        }
        info!(
            load_ms = ready.duration_ms.unwrap_or(0),
            "the whisper worker is ready"
        );
        Ok(Worker {
            child,
            stdin,
            stdout,
        })
    }
}

#[async_trait]
impl SttProvider for LocalStt {
    fn name(&self) -> String {
        "local".to_string()
    }

    async fn transcribe(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript> {
        let wav = std::env::temp_dir().join(format!("fcp-stt-{}.wav", uuid::Uuid::new_v4()));
        tokio::fs::write(&wav, wav_bytes(pcm)).await?;
        let result = self.ask(&wav, options).await;
        let _ = tokio::fs::remove_file(&wav).await;
        result
    }
}

impl LocalStt {
    async fn ask(&self, wav: &std::path::Path, options: &SttOptions) -> Result<Transcript> {
        let mut guard = self.worker.lock().await;
        if let Some(worker) = guard.as_mut() {
            if !matches!(worker.child.try_wait(), Ok(None)) {
                warn!("the whisper worker had exited; restarting it");
                *guard = None;
            }
        }
        if guard.is_none() {
            *guard = Some(self.start().await?);
        }
        let worker = guard.as_mut().expect("started above");
        let request = Request {
            wav: &wav.to_string_lossy(),
            language: options.language.as_deref().map(primary_language),
            prompt: (!options.hints.is_empty()).then(|| options.hints.join(", ")),
        };
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        let answer = async {
            worker.stdin.write_all(line.as_bytes()).await?;
            worker.stdin.flush().await?;
            let mut reply = String::new();
            worker.stdout.read_line(&mut reply).await?;
            anyhow::Ok(reply)
        }
        .await;
        let reply = match answer {
            Ok(reply) if !reply.is_empty() => reply,
            Ok(_) | Err(_) => {
                *guard = None;
                bail!("the whisper worker went away");
            }
        };
        let response: Response = serde_json::from_str(reply.trim())
            .with_context(|| format!("the whisper worker's answer: {}", reply.trim()))?;
        if response.ok != Some(true) {
            bail!(
                "the whisper worker failed: {}",
                response.error.unwrap_or_default()
            );
        }
        Ok(Transcript::final_text(
            response.text.unwrap_or_default().trim(),
            response.confidence,
        ))
    }
}
