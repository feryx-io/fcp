//! Speech-to-text (STT), shaped like [`crate::tts`].
//!
//! A provider recognises 16 kHz mono PCM ([`STT_SAMPLE_RATE`]) two ways:
//!
//! - **one-shot**, [`SttProvider::transcribe`]: a whole recording in, one
//!   final [`Transcript`] out (voicemail);
//! - **streaming**, [`SttService::stream`]: frames in as a call produces
//!   them, partial and final transcripts out (a flow listening). A provider
//!   that streams natively (Deepgram, `remote`) does so; one that does not
//!   (OpenAI, the local faster-whisper worker) is cut into utterances by
//!   the [`Endpointer`] and each utterance transcribed whole.
//!
//! The call's audio comes from [`crate::MediaManager::listen`].
//! `docs/DEVELOPER_SURFACE.md` phase 6.
//!
//! ```text
//!              SttService ── transcribe_wav / transcribe_pcm / stream
//!                  │
//!            SttProvider trait
//!      ┌──────────┬───┴───────┬──────────┐
//!  deepgram     openai     local      remote ── fcp-tts-server /v1/transcribe
//!  (stream)   (VAD-cut)  (VAD-cut)   (stream)
//! ```

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub mod providers;

/// The rate every provider is fed: 16 kHz, 16-bit, mono.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// How deep a stream's queues are: at 20 ms a frame, ten seconds of audio
/// a slow provider may fall behind by before frames are dropped.
const STREAM_QUEUE: usize = 500;

/// What a recognition is asked for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SttOptions {
    /// BCP 47 language (`en-US`); the service's default when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Words or phrases the speaker is likely to say, to bias recognition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    /// Streaming only: send partial transcripts as well as finals.
    #[serde(default)]
    pub interim_results: bool,
    /// Streaming only: the silence, in milliseconds, that ends an utterance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_timeout_ms: Option<u32>,
    /// Whose speech this is (`docs/TENANT_RECOGNITION.md` R1): the shared
    /// server (`remote`) hears it with the tenant's recognizer and in its
    /// language; any other provider ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl SttOptions {
    /// The end-of-utterance silence, defaulted.
    pub fn speech_timeout_ms(&self) -> u32 {
        self.speech_timeout_ms.unwrap_or(DEFAULT_SPEECH_TIMEOUT_MS)
    }
}

/// The default end-of-utterance silence.
pub const DEFAULT_SPEECH_TIMEOUT_MS: u32 = 800;

/// A recognised piece of speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// What was said.
    pub text: String,
    /// The provider's confidence, 0.0 to 1.0, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// A final transcript ends an utterance; a partial one may be revised.
    pub is_final: bool,
}

impl Transcript {
    /// A final transcript.
    pub fn final_text(text: impl Into<String>, confidence: Option<f64>) -> Self {
        Self {
            text: text.into(),
            confidence,
            is_final: true,
        }
    }
}

/// A streaming recognition: audio in, transcripts out.
///
/// Dropping the audio side ([`SttStream::finish`]) ends the utterance in
/// progress; the provider sends its last finals and the results end.
pub struct SttStream {
    audio: Option<mpsc::Sender<Vec<i16>>>,
    results: mpsc::Receiver<Transcript>,
    /// The task feeding the audio in, stopped with the stream.
    feeder: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for SttStream {
    fn drop(&mut self) {
        if let Some(feeder) = self.feeder.take() {
            feeder.abort();
        }
    }
}

impl SttStream {
    /// A stream over its two channels, for a provider to hand back.
    pub fn new(audio: mpsc::Sender<Vec<i16>>, results: mpsc::Receiver<Transcript>) -> Self {
        Self {
            audio: Some(audio),
            results,
            feeder: None,
        }
    }

    /// The two ends of a stream: the provider keeps the audio receiver and
    /// the results sender.
    pub fn channel() -> (Self, mpsc::Receiver<Vec<i16>>, mpsc::Sender<Transcript>) {
        let (audio_tx, audio_rx) = mpsc::channel(STREAM_QUEUE);
        let (results_tx, results_rx) = mpsc::channel(STREAM_QUEUE);
        (Self::new(audio_tx, results_rx), audio_rx, results_tx)
    }

    /// Feed 16 kHz mono PCM. Dropped, with a warning, when the provider is
    /// ten seconds behind; an error once the provider has gone.
    pub fn send(&self, pcm: Vec<i16>) -> Result<()> {
        let Some(audio) = &self.audio else {
            bail!("the stream is finished");
        };
        match audio.try_send(pcm) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("speech recognition is behind; a frame was dropped");
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => bail!("the recognizer has gone"),
        }
    }

    /// A sender for the audio side, for a task that pumps frames in.
    pub fn audio_sender(&self) -> Option<mpsc::Sender<Vec<i16>>> {
        self.audio.clone()
    }

    /// Feed the stream from a receiver of frames (a call's leg), until the
    /// frames end or the stream finishes.
    pub fn feed_from(&mut self, mut frames: mpsc::Receiver<Vec<i16>>) {
        let Some(audio) = self.audio.take() else {
            return;
        };
        if let Some(previous) = self.feeder.take() {
            previous.abort();
        }
        self.feeder = Some(tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                if audio.send(frame).await.is_err() {
                    return;
                }
            }
        }));
    }

    /// No more audio: the provider flushes and the results end.
    pub fn finish(&mut self) {
        self.audio = None;
        if let Some(feeder) = self.feeder.take() {
            feeder.abort();
        }
    }

    /// The next transcript, or `None` once the stream has ended.
    pub async fn next(&mut self) -> Option<Transcript> {
        self.results.recv().await
    }
}

/// A speech-to-text engine.
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// The provider's name (`deepgram`, `openai`, `local`, `remote`).
    fn name(&self) -> String;

    /// Recognise a whole recording of 16 kHz mono PCM.
    async fn transcribe(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript>;

    /// A native streaming recognition, or `None` when the provider has
    /// none and [`SttService::stream`] cuts the audio into utterances.
    async fn stream(&self, _options: &SttOptions) -> Result<Option<SttStream>> {
        Ok(None)
    }
}

/// Speech recognition configuration: `[stt]` in a binary's file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SttConfig {
    /// The engine.
    pub provider: SttProviderConfig,
    /// The language a recognition is in when it names none.
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "en-US".to_string()
}

/// Which engine recognises speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SttProviderConfig {
    /// Deepgram: streaming over a WebSocket, files over HTTPS.
    Deepgram {
        api_key: String,
        #[serde(default = "default_deepgram_model")]
        model: String,
        /// Another endpoint than `https://api.deepgram.com` (a proxy, a
        /// self-hosted Deepgram).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    /// OpenAI's transcription API: files, and a stream cut into
    /// utterances.
    #[serde(rename = "openai", alias = "open_ai")]
    OpenAi {
        api_key: String,
        #[serde(default = "default_openai_model")]
        model: String,
        /// Another endpoint than `https://api.openai.com` (a compatible
        /// server).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
    },
    /// faster-whisper in a local Python worker
    /// (`scripts/whisper_stt_worker.py`), like the kokoro TTS worker.
    Local {
        /// The Python interpreter; `python3` when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        python_path: Option<PathBuf>,
        /// The whisper model (`tiny`, `base`, `small`, `medium`,
        /// `large-v3`); `base` when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// `auto`, `cpu`, `cuda`; `auto` when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
    },
    /// The shared `fcp-tts-server`'s `/v1/transcribe`.
    Remote {
        url: String,
        auth_token: String,
        /// The server's provider by name; its default when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider: Option<String>,
    },
}

fn default_deepgram_model() -> String {
    "nova-3".to_string()
}

fn default_openai_model() -> String {
    "whisper-1".to_string()
}

/// Speech recognition over one provider.
pub struct SttService {
    provider: Arc<dyn SttProvider>,
    language: String,
}

impl SttService {
    /// The service the configuration names.
    pub async fn new(config: SttConfig) -> Result<Self> {
        let provider = providers::create_provider(config.provider).await?;
        Ok(Self::from_provider(provider, config.language))
    }

    /// The service over a provider already made.
    pub fn from_provider(provider: Arc<dyn SttProvider>, language: impl Into<String>) -> Self {
        Self {
            provider,
            language: language.into(),
        }
    }

    /// The provider's name.
    pub fn provider_name(&self) -> String {
        self.provider.name()
    }

    /// The default language.
    pub fn language(&self) -> &str {
        &self.language
    }

    fn with_language(&self, options: &SttOptions) -> SttOptions {
        let mut options = options.clone();
        // The shared server picks a tenant's language itself.
        let resolved_there = options.tenant_id.is_some() && self.provider.name() == "remote";
        if options.language.is_none() && !resolved_there {
            options.language = Some(self.language.clone());
        }
        options
    }

    /// Recognise 16 kHz mono PCM.
    pub async fn transcribe_pcm(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript> {
        self.provider
            .transcribe(pcm, &self.with_language(options))
            .await
    }

    /// Recognise a WAV file of any rate and channel count.
    pub async fn transcribe_wav(&self, path: &Path, options: &SttOptions) -> Result<Transcript> {
        let path = path.to_path_buf();
        let pcm = tokio::task::spawn_blocking(move || read_wav_16k_mono(&path))
            .await
            .context("reading the recording")??;
        self.transcribe_pcm(&pcm, options).await
    }

    /// A streaming recognition: the provider's own, or one cut into
    /// utterances by the [`Endpointer`] and each transcribed whole.
    pub async fn stream(&self, options: &SttOptions) -> Result<SttStream> {
        let options = self.with_language(options);
        if let Some(stream) = self.provider.stream(&options).await? {
            return Ok(stream);
        }
        Ok(utterance_stream(Arc::clone(&self.provider), options))
    }
}

/// A stream for a provider without one: utterances cut by energy, each
/// recognised whole and sent as a final transcript.
fn utterance_stream(provider: Arc<dyn SttProvider>, options: SttOptions) -> SttStream {
    let (stream, mut audio, results) = SttStream::channel();
    tokio::spawn(async move {
        let mut endpointer = Endpointer::new(options.speech_timeout_ms());
        loop {
            let utterance = match audio.recv().await {
                Some(frame) => match endpointer.push(&frame) {
                    Some(utterance) => utterance,
                    None => continue,
                },
                None => match endpointer.flush() {
                    Some(utterance) => {
                        send_utterance(&*provider, &utterance, &options, &results).await;
                        return;
                    }
                    None => return,
                },
            };
            send_utterance(&*provider, &utterance, &options, &results).await;
        }
    });
    stream
}

async fn send_utterance(
    provider: &dyn SttProvider,
    utterance: &[i16],
    options: &SttOptions,
    results: &mpsc::Sender<Transcript>,
) {
    match provider.transcribe(utterance, options).await {
        Ok(transcript) if !transcript.text.trim().is_empty() => {
            let _ = results.send(transcript).await;
        }
        Ok(_) => debug!("an utterance recognised as nothing"),
        Err(e) => warn!(provider = %provider.name(), error = %e, "speech recognition failed"),
    }
}

/// Cuts 16 kHz audio into utterances by energy: an utterance starts at a
/// frame above the threshold and ends after `silence_ms` below it (or at
/// thirty seconds). A burst shorter than 100 ms is noise, not speech.
pub struct Endpointer {
    silence_samples: usize,
    buffer: Vec<i16>,
    voiced_samples: usize,
    trailing_silence: usize,
    in_speech: bool,
}

/// RMS above which a frame is speech: -40 dBFS.
const SPEECH_RMS: f64 = 328.0;
/// Audio kept from before the first voiced frame, so a soft onset is heard.
const PRE_ROLL_SAMPLES: usize = STT_SAMPLE_RATE as usize / 5;
/// The shortest burst that counts as speech.
const MIN_SPEECH_SAMPLES: usize = STT_SAMPLE_RATE as usize / 10;
/// The longest utterance before it is cut regardless.
const MAX_UTTERANCE_SAMPLES: usize = STT_SAMPLE_RATE as usize * 30;

impl Endpointer {
    /// An endpointer ending an utterance after `silence_ms` of silence.
    pub fn new(silence_ms: u32) -> Self {
        Self {
            silence_samples: (STT_SAMPLE_RATE as usize * silence_ms as usize) / 1000,
            buffer: Vec::new(),
            voiced_samples: 0,
            trailing_silence: 0,
            in_speech: false,
        }
    }

    /// Feed a frame; an utterance when this frame ended one.
    pub fn push(&mut self, frame: &[i16]) -> Option<Vec<i16>> {
        if frame.is_empty() {
            return None;
        }
        let voiced = rms(frame) >= SPEECH_RMS;
        if !self.in_speech {
            self.buffer.extend_from_slice(frame);
            if voiced {
                self.in_speech = true;
                self.voiced_samples = frame.len();
                self.trailing_silence = 0;
            } else if self.buffer.len() > PRE_ROLL_SAMPLES {
                let excess = self.buffer.len() - PRE_ROLL_SAMPLES;
                self.buffer.drain(..excess);
            }
            return None;
        }
        self.buffer.extend_from_slice(frame);
        if voiced {
            self.voiced_samples += frame.len();
            self.trailing_silence = 0;
        } else {
            self.trailing_silence += frame.len();
        }
        if self.trailing_silence >= self.silence_samples
            || self.buffer.len() >= MAX_UTTERANCE_SAMPLES
        {
            return self.take();
        }
        None
    }

    /// The utterance in progress, when the audio ends.
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        if self.in_speech {
            self.take()
        } else {
            None
        }
    }

    /// Whether speech has started and not yet ended.
    pub fn in_speech(&self) -> bool {
        self.in_speech
    }

    fn take(&mut self) -> Option<Vec<i16>> {
        let voiced = self.voiced_samples;
        self.in_speech = false;
        self.voiced_samples = 0;
        self.trailing_silence = 0;
        let utterance = std::mem::take(&mut self.buffer);
        (voiced >= MIN_SPEECH_SAMPLES).then_some(utterance)
    }
}

/// Whether a frame is loud enough to be speech (the [`Endpointer`]'s
/// threshold).
pub fn is_voiced(frame: &[i16]) -> bool {
    !frame.is_empty() && rms(frame) >= SPEECH_RMS
}

/// The root mean square of a frame.
pub fn rms(frame: &[i16]) -> f64 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / frame.len() as f64).sqrt()
}

/// PCM at `rate` brought to 16 kHz.
pub fn resample_to_16k(pcm: &[i16], rate: u32) -> Result<Vec<i16>> {
    if rate == STT_SAMPLE_RATE || pcm.is_empty() {
        return Ok(pcm.to_vec());
    }
    forge_resampler::resample_all(pcm, rate, STT_SAMPLE_RATE, 1)
        .map_err(|e| anyhow::anyhow!("resampling {rate} Hz: {e}"))
}

/// A WAV file as 16 kHz mono PCM: channels averaged, integer or float
/// samples, any rate.
pub fn read_wav_16k_mono(path: &Path) -> Result<Vec<i16>> {
    let reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    wav_16k_mono(reader)
}

/// A WAV file's bytes as 16 kHz mono PCM, as [`read_wav_16k_mono`].
pub fn wav_bytes_16k_mono(bytes: &[u8]) -> Result<Vec<i16>> {
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).context("reading the WAV")?;
    wav_16k_mono(reader)
}

fn wav_16k_mono<R: std::io::Read>(mut reader: hound::WavReader<R>) -> Result<Vec<i16>> {
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let samples: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int if spec.bits_per_sample <= 16 => {
            let shift = 16 - spec.bits_per_sample as u32;
            reader
                .samples::<i16>()
                .map(|s| s.map(|s| s << shift))
                .collect::<std::result::Result<_, _>>()?
        }
        hound::SampleFormat::Int => {
            let shift = spec.bits_per_sample as u32 - 16;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| (s >> shift) as i16))
                .collect::<std::result::Result<_, _>>()?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16))
            .collect::<std::result::Result<_, _>>()?,
    };
    let mono: Vec<i16> = if channels == 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|frame| (frame.iter().map(|&s| s as i32).sum::<i32>() / channels as i32) as i16)
            .collect()
    };
    resample_to_16k(&mono, spec.sample_rate)
}

/// 16 kHz mono PCM as a WAV file's bytes, for providers that take files.
pub fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&STT_SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(STT_SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Little-endian 16-bit PCM bytes as samples (a trailing odd byte dropped).
pub fn pcm_from_le_bytes(bytes: &[u8]) -> Vec<i16> {
    let (pairs, _) = bytes.as_chunks::<2>();
    pairs.iter().map(|&b| i16::from_le_bytes(b)).collect()
}

/// Samples as little-endian 16-bit PCM bytes.
pub fn pcm_to_le_bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// The ISO 639-1 part of a BCP 47 tag (`en-US` → `en`), which the
/// whisper models take.
pub fn primary_language(language: &str) -> &str {
    language.split(['-', '_']).next().unwrap_or(language)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(ms: usize, amplitude: f64) -> Vec<i16> {
        let n = STT_SAMPLE_RATE as usize * ms / 1000;
        (0..n)
            .map(|i| {
                (amplitude * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / 16_000.0).sin())
                    as i16
            })
            .collect()
    }

    fn frames(pcm: &[i16]) -> impl Iterator<Item = &[i16]> {
        pcm.chunks(320)
    }

    #[test]
    fn an_utterance_ends_after_the_silence() {
        let mut endpointer = Endpointer::new(500);
        let mut audio = tone(200, 0.0);
        audio.extend(tone(600, 8000.0));
        audio.extend(tone(700, 0.0));
        let mut cut = Vec::new();
        for frame in frames(&audio) {
            if let Some(u) = endpointer.push(frame) {
                cut.push(u);
            }
        }
        assert_eq!(cut.len(), 1, "one utterance");
        // Pre-roll, the speech, and the silence that ended it.
        let len = cut[0].len();
        assert!(len >= 16 * (200 + 600 + 500) - 320, "{len}");
        assert!(endpointer.flush().is_none());
    }

    #[test]
    fn a_click_is_not_speech_and_the_end_of_audio_flushes() {
        let mut endpointer = Endpointer::new(500);
        let mut audio = tone(40, 8000.0);
        audio.extend(tone(600, 0.0));
        for frame in frames(&audio) {
            assert!(endpointer.push(frame).is_none());
        }
        for frame in frames(&tone(300, 8000.0)) {
            assert!(endpointer.push(frame).is_none());
        }
        assert!(endpointer.in_speech());
        assert!(endpointer.flush().is_some());
    }

    #[test]
    fn a_wav_is_read_as_16k_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo8k.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..8000 {
            writer.write_sample(1000i16).unwrap();
            writer.write_sample(3000i16).unwrap();
        }
        writer.finalize().unwrap();
        let pcm = read_wav_16k_mono(&path).unwrap();
        assert!((15_900..=16_100).contains(&pcm.len()), "{}", pcm.len());
        assert!((1900..=2100).contains(&pcm[8000]), "{}", pcm[8000]);
    }

    #[test]
    fn wav_bytes_round_trip() {
        let pcm: Vec<i16> = (0..1600).map(|i| i as i16).collect();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, wav_bytes(&pcm)).unwrap();
        assert_eq!(read_wav_16k_mono(&path).unwrap(), pcm);
        assert_eq!(pcm_from_le_bytes(&pcm_to_le_bytes(&pcm)), pcm);
    }

    #[test]
    fn the_provider_configuration_reads_and_refuses_unknown_keys() {
        let config: SttConfig = toml::from_str(
            r#"language = "en-GB"
[provider]
type = "deepgram"
api_key = "dg"
"#,
        )
        .unwrap();
        assert_eq!(config.language, "en-GB");
        assert_eq!(
            config.provider,
            SttProviderConfig::Deepgram {
                api_key: "dg".into(),
                model: "nova-3".into(),
                base_url: None
            }
        );
        let openai: SttProviderConfig =
            toml::from_str("type = \"openai\"\napi_key = \"k\"").unwrap();
        assert!(matches!(openai, SttProviderConfig::OpenAi { .. }));
        assert!(toml::from_str::<SttProviderConfig>(
            "type = \"remote\"\nurl = \"http://x\"\nauth_token = \"t\"\nvoice = \"x\""
        )
        .is_err());
    }

    struct Echo;

    #[async_trait]
    impl SttProvider for Echo {
        fn name(&self) -> String {
            "echo".into()
        }
        async fn transcribe(&self, pcm: &[i16], options: &SttOptions) -> Result<Transcript> {
            Ok(Transcript::final_text(
                format!(
                    "{} ms in {}",
                    pcm.len() / 16,
                    options.language.as_deref().unwrap_or("?")
                ),
                Some(0.5),
            ))
        }
    }

    #[tokio::test]
    async fn a_provider_without_a_stream_is_cut_into_utterances() {
        let service = SttService::from_provider(Arc::new(Echo), "fr-FR");
        let mut stream = service
            .stream(&SttOptions {
                speech_timeout_ms: Some(300),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut audio = tone(500, 8000.0);
        audio.extend(tone(400, 0.0));
        audio.extend(tone(200, 8000.0));
        for frame in frames(&audio) {
            stream.send(frame.to_vec()).unwrap();
        }
        let first = stream.next().await.unwrap();
        assert!(first.is_final);
        assert!(first.text.ends_with("in fr-FR"), "{}", first.text);
        stream.finish();
        let second = stream.next().await.unwrap();
        // The 100 ms of silence after the first is the second's pre-roll.
        assert!(second.text.starts_with("300 ms"), "{}", second.text);
        assert!(stream.next().await.is_none());
    }
}
