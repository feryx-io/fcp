//! Local TTS provider using Kokoro (persistent Python subprocess).
//!
//! Spawns `kokoro_tts_worker.py` as a long-lived child process that keeps
//! the Kokoro model loaded in memory.  Communication is JSONL-over-stdio:
//!
//! ```text
//! stdin:  {"text":"Hello","voice":"af_heart","lang":"a","output":"/tmp/out.wav"}\n
//! stdout: {"ok":true,"duration_ms":850}\n
//! ```

use super::super::{TtsProvider, VoiceInfo};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Kokoro voice catalogue (a representative subset).
const KOKORO_VOICES: &[(&str, &str, &str)] = &[
    ("af_heart", "Heart (American Female)", "en-US"),
    ("af_bella", "Bella (American Female)", "en-US"),
    ("af_nicole", "Nicole (American Female)", "en-US"),
    ("af_sarah", "Sarah (American Female)", "en-US"),
    ("af_sky", "Sky (American Female)", "en-US"),
    ("am_adam", "Adam (American Male)", "en-US"),
    ("am_michael", "Michael (American Male)", "en-US"),
    ("bf_emma", "Emma (British Female)", "en-GB"),
    ("bf_isabella", "Isabella (British Female)", "en-GB"),
    ("bm_george", "George (British Male)", "en-GB"),
    ("bm_lewis", "Lewis (British Male)", "en-GB"),
];

/// Local TTS provider backed by a persistent Kokoro Python subprocess.
pub struct LocalTts {
    engine: String,
    python_path: PathBuf,
    worker_script: PathBuf,
    worker: Mutex<Option<KokoroWorker>>,
    default_voice: String,
}

/// Handle to the running Python worker subprocess.
struct KokoroWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// JSON request sent to the worker.
#[derive(Serialize)]
struct WorkerRequest {
    text: String,
    voice: String,
    lang: String,
    output: String,
}

/// JSON response received from the worker.
#[derive(Deserialize)]
struct WorkerResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    ready: Option<bool>,
}

impl LocalTts {
    /// Create a new local TTS provider.
    ///
    /// Only `engine = "kokoro"` is supported.  `executable_path` optionally
    /// overrides the Python interpreter (defaults to `python3`).
    pub fn new(engine: String, executable_path: Option<PathBuf>) -> Result<Self> {
        if engine != "kokoro" {
            bail!(
                "Unsupported local TTS engine '{}'. Only 'kokoro' is supported.",
                engine
            );
        }

        let python_path = executable_path.unwrap_or_else(|| PathBuf::from("python3"));

        // Resolve worker script location:
        // 1. $FCP_KOKORO_WORKER env var
        // 2. Relative to current executable: ../../../scripts/kokoro_tts_worker.py
        // 3. ./scripts/kokoro_tts_worker.py (development layout)
        let worker_script = if let Ok(path) = std::env::var("FCP_KOKORO_WORKER") {
            PathBuf::from(path)
        } else if let Ok(exe) = std::env::current_exe() {
            // Binary is at target/debug/fcp-call-manager or similar
            let candidate = exe
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.join("scripts/kokoro_tts_worker.py"));
            if let Some(ref c) = candidate {
                if c.exists() {
                    c.clone()
                } else {
                    PathBuf::from("scripts/kokoro_tts_worker.py")
                }
            } else {
                PathBuf::from("scripts/kokoro_tts_worker.py")
            }
        } else {
            PathBuf::from("scripts/kokoro_tts_worker.py")
        };

        info!(
            engine = %engine,
            python = %python_path.display(),
            worker = %worker_script.display(),
            "Local TTS provider created (worker will start on first synthesis)"
        );

        Ok(Self {
            engine,
            python_path,
            worker_script,
            worker: Mutex::new(None),
            default_voice: "af_heart".to_string(),
        })
    }

    /// Ensure the worker subprocess is running, spawning it if needed.
    async fn ensure_worker(&self) -> Result<()> {
        let mut guard = self.worker.lock().await;
        if guard.is_some() {
            // Check if child is still alive
            if let Some(ref mut w) = *guard {
                match w.child.try_wait() {
                    Ok(Some(status)) => {
                        warn!(exit_code = ?status.code(), "Kokoro worker exited unexpectedly, restarting");
                        *guard = None;
                    }
                    Ok(None) => return Ok(()), // still running
                    Err(e) => {
                        warn!(error = %e, "Failed to check worker status, restarting");
                        *guard = None;
                    }
                }
            }
        }

        // Spawn new worker
        info!(
            script = %self.worker_script.display(),
            python = %self.python_path.display(),
            "Starting Kokoro TTS worker subprocess"
        );

        let mut child = Command::new(&self.python_path)
            .arg(&self.worker_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit()) // let stderr go to parent's stderr
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to spawn Kokoro worker: {} {}",
                    self.python_path.display(),
                    self.worker_script.display()
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .context("Failed to capture worker stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("Failed to capture worker stdout")?;
        let mut stdout = BufReader::new(stdout);

        // Wait for the "ready" message from the worker
        let mut ready_line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(120), // model loading can take a while
            stdout.read_line(&mut ready_line),
        )
        .await
        .context("Timeout waiting for Kokoro worker to start (model download may be needed)")?
        .context("Failed to read ready message from Kokoro worker")?;

        let ready_resp: WorkerResponse = serde_json::from_str(ready_line.trim())
            .context("Failed to parse worker ready message")?;

        if ready_resp.ready != Some(true) {
            bail!(
                "Worker did not send ready signal: {}",
                ready_line.trim()
            );
        }

        info!(
            load_ms = ready_resp.duration_ms.unwrap_or(0),
            "Kokoro TTS worker is ready"
        );

        *guard = Some(KokoroWorker {
            child,
            stdin,
            stdout,
        });

        Ok(())
    }
}

#[async_trait]
impl TtsProvider for LocalTts {
    fn name(&self) -> String {
        format!("local_{}", self.engine)
    }

    async fn synthesize(
        &self,
        text: &str,
        language: &str,
        voice: Option<&str>,
    ) -> Result<Vec<u8>> {
        self.ensure_worker().await?;

        let voice = voice.unwrap_or(&self.default_voice);

        // Create a temp file for the worker to write to
        let tmp_dir = std::env::temp_dir();
        let output_path = tmp_dir.join(format!("kokoro_{}.wav", uuid::Uuid::new_v4()));

        let request = WorkerRequest {
            text: text.to_string(),
            voice: voice.to_string(),
            lang: language.to_string(),
            output: output_path.to_string_lossy().to_string(),
        };

        let request_json =
            serde_json::to_string(&request).context("Failed to serialize TTS request")?;

        debug!(
            text_len = text.len(),
            voice = %voice,
            lang = %language,
            "Sending synthesis request to Kokoro worker"
        );

        // Send request and read response while holding the lock
        let response = {
            let mut guard = self.worker.lock().await;
            let worker = guard
                .as_mut()
                .context("Kokoro worker not running (should not happen after ensure_worker)")?;

            // Write request
            worker
                .stdin
                .write_all(request_json.as_bytes())
                .await
                .context("Failed to write to Kokoro worker stdin")?;
            worker
                .stdin
                .write_all(b"\n")
                .await
                .context("Failed to write newline to worker")?;
            worker
                .stdin
                .flush()
                .await
                .context("Failed to flush worker stdin")?;

            // Read response
            let mut response_line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(60),
                worker.stdout.read_line(&mut response_line),
            )
            .await
            .context("Timeout waiting for Kokoro synthesis response")?
            .context("Failed to read synthesis response")?;

            let resp: WorkerResponse = serde_json::from_str(response_line.trim())
                .context("Failed to parse worker response")?;

            resp
        };

        if !response.ok {
            let error_msg = response.error.unwrap_or_else(|| "Unknown error".to_string());
            bail!("Kokoro synthesis failed: {}", error_msg);
        }

        debug!(
            duration_ms = response.duration_ms.unwrap_or(0),
            output = %output_path.display(),
            "Kokoro synthesis complete"
        );

        // Read the generated WAV file
        let audio_data = tokio::fs::read(&output_path)
            .await
            .with_context(|| format!("Failed to read generated audio: {}", output_path.display()))?;

        // Clean up temp file
        let _ = tokio::fs::remove_file(&output_path).await;

        Ok(audio_data)
    }

    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        let voices: Vec<VoiceInfo> = KOKORO_VOICES
            .iter()
            .filter(|(_, _, lang)| language.is_empty() || lang == &language)
            .map(|(name, display, lang)| VoiceInfo {
                name: name.to_string(),
                display_name: display.to_string(),
                language: lang.to_string(),
                gender: if name.contains("af_") || name.contains("bf_") {
                    Some("female".to_string())
                } else {
                    Some("male".to_string())
                },
                metadata: HashMap::new(),
            })
            .collect();

        Ok(voices)
    }
}

impl Drop for LocalTts {
    fn drop(&mut self) {
        // KokoroWorker's Child has kill_on_drop(true), so it will be killed
        // automatically when dropped.
        debug!("LocalTts provider dropped, worker will be terminated");
    }
}
