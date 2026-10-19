//! Piper, the default local TTS engine (`docs/SPEECH.md`).
//!
//! Piper is a small native engine (ONNX voices of about 60 MB, faster than
//! real time on a CPU) with none of Kokoro's Python and PyTorch, so it can
//! run in every stack: without an engine, voicemail, greetings and every
//! prompt were silence (`docs/DEEP_TESTING.md` Q-D8a).
//!
//! One `piper --json-input` process per voice, started when the voice is
//! first asked for and kept: a line of JSON in, the WAV written, its path
//! printed back.
//!
//! ```text
//! stdin:  {"text":"Hello","output_file":"/tmp/piper_….wav"}
//! stdout: /tmp/piper_….wav
//! ```
//!
//! Piper writes at its voice's rate (22.05 kHz for most); what is returned
//! is 8 kHz mono, as every provider's is.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// How long one sentence may take before the voice's process is replaced.
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);

/// The rate every provider returns.
const OUTPUT_RATE: u32 = 8000;

/// Piper with the voices in a directory.
pub struct PiperTts {
    binary: PathBuf,
    voices_dir: PathBuf,
    default_voice: String,
    workers: Mutex<HashMap<String, PiperWorker>>,
}

struct PiperWorker {
    // Held for kill_on_drop.
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl PiperTts {
    /// Piper at `binary` (or `piper` on the PATH), its voices the `.onnx`
    /// models in `voices_dir`, `default_voice` spoken when a request names
    /// none and no voice speaks its language.
    pub fn new(
        binary: Option<PathBuf>,
        voices_dir: PathBuf,
        default_voice: String,
    ) -> Result<Self> {
        let binary = binary.unwrap_or_else(|| PathBuf::from("piper"));
        let tts = Self {
            binary,
            voices_dir,
            default_voice,
            workers: Mutex::new(HashMap::new()),
        };
        if !tts.model(&tts.default_voice).exists() {
            bail!(
                "Piper's default voice {} is not in {}",
                tts.default_voice,
                tts.voices_dir.display()
            );
        }
        info!(
            binary = %tts.binary.display(),
            voices = %tts.voices_dir.display(),
            default_voice = %tts.default_voice,
            "Piper TTS provider created"
        );
        Ok(tts)
    }

    /// Start the default voice now, so the first prompt does not wait.
    pub async fn warm_up(&self) -> Result<()> {
        let mut workers = self.workers.lock().await;
        if !workers.contains_key(&self.default_voice) {
            let worker = self.spawn(&self.default_voice)?;
            workers.insert(self.default_voice.clone(), worker);
        }
        Ok(())
    }

    fn model(&self, voice: &str) -> PathBuf {
        self.voices_dir.join(format!("{voice}.onnx"))
    }

    /// The voices installed: every model with its config beside it.
    fn installed(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.voices_dir) else {
            return Vec::new();
        };
        let mut voices: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "onnx"))
            .filter(|p| p.with_extension("onnx.json").exists())
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .collect();
        voices.sort();
        voices
    }

    /// The voice to speak with: the one asked for if installed, else one
    /// of the language's (`en-US` → `en_US-…`), else the default.
    fn choose(&self, language: &str, voice: Option<&str>) -> String {
        let installed = self.installed();
        if let Some(voice) = voice.filter(|v| installed.iter().any(|i| i == v)) {
            return voice.to_string();
        }
        let prefix = format!("{}-", language.replace('-', "_"));
        if self.default_voice.starts_with(&prefix) {
            return self.default_voice.clone();
        }
        installed
            .into_iter()
            .find(|v| v.starts_with(&prefix))
            .unwrap_or_else(|| self.default_voice.clone())
    }

    fn spawn(&self, voice: &str) -> Result<PiperWorker> {
        let model = self.model(voice);
        let mut child = Command::new(&self.binary)
            .arg("--model")
            .arg(&model)
            .arg("--json-input")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {} for {}", self.binary.display(), voice))?;
        let stdin = child.stdin.take().context("Piper's stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("Piper's stdout")?);
        debug!(voice, model = %model.display(), "Piper voice started");
        Ok(PiperWorker {
            _child: child,
            stdin,
            stdout,
        })
    }

    async fn speak(&self, voice: &str, text: &str, output: &Path) -> Result<()> {
        let request = serde_json::json!({
            "text": text,
            "output_file": output.to_string_lossy(),
        })
        .to_string();
        let mut workers = self.workers.lock().await;
        if !workers.contains_key(voice) {
            let worker = self.spawn(voice)?;
            workers.insert(voice.to_string(), worker);
        }
        let worker = workers.get_mut(voice).expect("inserted above");
        let exchange = async {
            worker.stdin.write_all(request.as_bytes()).await?;
            worker.stdin.write_all(b"\n").await?;
            worker.stdin.flush().await?;
            let mut line = String::new();
            let n = worker.stdout.read_line(&mut line).await?;
            anyhow::Ok((n, line))
        };
        match tokio::time::timeout(SYNTHESIS_TIMEOUT, exchange).await {
            Ok(Ok((n, _))) if n > 0 => Ok(()),
            Ok(Ok(_)) => {
                workers.remove(voice);
                bail!("Piper ({voice}) exited")
            }
            Ok(Err(e)) => {
                workers.remove(voice);
                Err(e).context(format!("talking to Piper ({voice})"))
            }
            Err(_) => {
                warn!(voice, "Piper took too long; its process is replaced");
                workers.remove(voice);
                bail!("Piper ({voice}) took longer than {SYNTHESIS_TIMEOUT:?}")
            }
        }
    }
}

/// A WAV at any rate as 8 kHz mono 16-bit WAV.
fn to_8k_wav(bytes: &[u8]) -> Result<Vec<u8>> {
    let reader =
        hound::WavReader::new(std::io::Cursor::new(bytes)).context("reading Piper's WAV")?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<std::result::Result<_, _>>()
        .context("reading Piper's samples")?;
    let mono: Vec<i16> = samples
        .chunks(channels)
        .map(|c| (c.iter().map(|&s| i32::from(s)).sum::<i32>() / c.len() as i32) as i16)
        .collect();
    let pcm = if spec.sample_rate == OUTPUT_RATE {
        mono
    } else {
        forge_resampler::Resampler::new(spec.sample_rate, OUTPUT_RATE, 1)
            .map_err(|e| anyhow::anyhow!("resampling {} Hz: {e}", spec.sample_rate))?
            .resample(&mono)
            .map_err(|e| anyhow::anyhow!("resampling {} Hz: {e}", spec.sample_rate))?
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut out,
        hound::WavSpec {
            channels: 1,
            sample_rate: OUTPUT_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for s in pcm {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(out.into_inner())
}

#[async_trait]
impl TtsProvider for PiperTts {
    fn name(&self) -> String {
        "piper".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        let voice = self.choose(language, voice);
        let output = std::env::temp_dir().join(format!("piper_{}.wav", uuid::Uuid::new_v4()));
        let spoken = self.speak(&voice, text, &output).await;
        let bytes = match spoken {
            Ok(()) => tokio::fs::read(&output)
                .await
                .with_context(|| format!("reading {}", output.display())),
            Err(e) => Err(e),
        };
        let _ = tokio::fs::remove_file(&output).await;
        to_8k_wav(&bytes?)
    }

    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        let prefix = language.replace('-', "_");
        Ok(self
            .installed()
            .into_iter()
            .filter(|v| language.is_empty() || v.starts_with(&prefix))
            .map(|v| VoiceInfo {
                language: v.split('-').next().unwrap_or("").replace('_', "-"),
                display_name: v.clone(),
                name: v,
                gender: None,
                metadata: HashMap::new(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A voices directory with these models (empty files; only names).
    fn voices(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            std::fs::write(dir.path().join(format!("{name}.onnx")), b"").unwrap();
            std::fs::write(dir.path().join(format!("{name}.onnx.json")), b"{}").unwrap();
        }
        dir
    }

    #[test]
    fn a_voice_is_the_one_named_else_the_languages_else_the_default() {
        let dir = voices(&[
            "en_US-ljspeech-medium",
            "en_GB-cori-high",
            "de_DE-thorsten-medium",
        ]);
        let tts = PiperTts::new(None, dir.path().into(), "en_US-ljspeech-medium".into()).unwrap();
        assert_eq!(
            tts.choose("en-US", Some("en_GB-cori-high")),
            "en_GB-cori-high"
        );
        assert_eq!(tts.choose("en-GB", None), "en_GB-cori-high");
        assert_eq!(tts.choose("de-DE", None), "de_DE-thorsten-medium");
        assert_eq!(tts.choose("fr-FR", None), "en_US-ljspeech-medium");
        assert_eq!(tts.choose("en-US", Some("nobody")), "en_US-ljspeech-medium");
    }

    #[test]
    fn a_missing_default_voice_is_refused() {
        let dir = voices(&["en_GB-cori-high"]);
        assert!(PiperTts::new(None, dir.path().into(), "en_US-ljspeech-medium".into()).is_err());
    }

    #[test]
    fn audio_at_any_rate_comes_back_at_8k() {
        let mut wav = std::io::Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(
            &mut wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: 22050,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..22050 {
            w.write_sample(((i as f32 * 0.05).sin() * 8000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
        let out = to_8k_wav(&wav.into_inner()).unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(out)).unwrap();
        assert_eq!(reader.spec().sample_rate, 8000);
        let n = reader.len() as i64;
        assert!((n - 8000).abs() < 200, "a second at 8 kHz: {n} samples");
    }

    /// Writes `FCP_PIPER_RECORD`'s text, spoken by the installed engine, to
    /// `FCP_PIPER_RECORD_TO`: how the stack's default greeting was made
    /// (`deploy/docker-compose/audio`).
    #[tokio::test]
    async fn record_a_prompt_when_asked() {
        let (Ok(binary), Ok(voices), Ok(text), Ok(to)) = (
            std::env::var("FCP_PIPER"),
            std::env::var("FCP_PIPER_VOICES"),
            std::env::var("FCP_PIPER_RECORD"),
            std::env::var("FCP_PIPER_RECORD_TO"),
        ) else {
            return;
        };
        let tts = PiperTts::new(
            Some(binary.into()),
            voices.into(),
            "en_US-ljspeech-medium".into(),
        )
        .unwrap();
        let wav = tts.synthesize(&text, "en-US", None).await.unwrap();
        std::fs::write(to, wav).unwrap();
    }

    /// The real engine, when it is installed: `FCP_PIPER` (the binary) and
    /// `FCP_PIPER_VOICES` (a directory with `en_US-ljspeech-medium`).
    #[tokio::test]
    async fn piper_speaks_a_prompt_when_installed() {
        let (Ok(binary), Ok(voices)) = (
            std::env::var("FCP_PIPER"),
            std::env::var("FCP_PIPER_VOICES"),
        ) else {
            eprintln!("FCP_PIPER / FCP_PIPER_VOICES not set; skipping");
            return;
        };
        let tts = PiperTts::new(
            Some(binary.into()),
            voices.into(),
            "en_US-ljspeech-medium".into(),
        )
        .unwrap();
        tts.warm_up().await.unwrap();
        let wav = tts
            .synthesize("Please leave a message after the tone.", "en-US", None)
            .await
            .unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 8000);
        assert!(
            reader.len() > 8000,
            "over a second of speech: {}",
            reader.len()
        );
        // A second request reuses the voice's process.
        let again = tts.synthesize("Goodbye.", "en-US", None).await.unwrap();
        assert!(again.len() > 1000);
    }
}
