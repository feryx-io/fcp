//! ElevenLabs text to speech (#339).
//!
//! - `POST /v1/text-to-speech/{voice_id}?output_format=pcm_16000`, the key
//!   in `xi-api-key`: raw 16-bit PCM at 16 kHz, returned at 8 kHz like
//!   every provider's audio.
//! - `GET /v1/voices`: the account's voices.
//!
//! <https://elevenlabs.io/docs/api-reference/text-to-speech/convert>
//!
//! A voice is named by its id or by its name in the account. The model is
//! `eleven_flash_v2_5` unless configured; the v2.5 models are told the
//! request's language, which the others refuse.

use super::super::{TtsProvider, VoiceInfo};
use super::{language_part, pcm_to_8k_wav, vendor_error};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, warn};

const API_URL: &str = "https://api.elevenlabs.io";
const DEFAULT_MODEL: &str = "eleven_flash_v2_5";
/// Rachel, one of the voices every account has.
const DEFAULT_VOICE: &str = "21m00Tcm4TlvDq8ikWAM";
/// The rate asked for: offered on every plan.
const RATE: u32 = 16_000;

pub struct ElevenLabsTts {
    api_key: String,
    model_id: String,
    base: String,
    client: reqwest::Client,
}

impl ElevenLabsTts {
    pub fn new(
        api_key: String,
        model_id: Option<String>,
        endpoint: Option<String>,
    ) -> Result<Self> {
        if api_key.trim().is_empty() {
            bail!("ElevenLabs TTS provider requires a non-empty API key");
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Self {
            api_key,
            model_id: model_id
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            base: endpoint
                .unwrap_or_else(|| API_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            client,
        })
    }

    /// The voice's id: `voice` itself when it is one, else the account's
    /// voice of that name, else the default.
    async fn voice_id(&self, voice: Option<&str>) -> String {
        let Some(voice) = voice.filter(|v| !v.trim().is_empty()) else {
            return DEFAULT_VOICE.to_string();
        };
        if is_voice_id(voice) {
            return voice.to_string();
        }
        match self.voices().await {
            Ok(voices) => {
                if let Some(found) = voices.iter().find(|v| v.name.eq_ignore_ascii_case(voice)) {
                    return found.voice_id.clone();
                }
                warn!(voice, "No ElevenLabs voice of that name; using the default");
            }
            Err(e) => {
                warn!(voice, error = %e, "Could not list ElevenLabs voices; using the default")
            }
        }
        DEFAULT_VOICE.to_string()
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        let response = self
            .client
            .get(format!("{}/v1/voices", self.base))
            .header("xi-api-key", &self.api_key)
            .send()
            .await
            .context("Failed to reach ElevenLabs")?;
        if !response.status().is_success() {
            return Err(vendor_error("ElevenLabs", response).await);
        }
        let list: VoiceList = response.json().await.context("ElevenLabs' voice list")?;
        Ok(list.voices)
    }
}

/// An ElevenLabs voice id: twenty letters and digits.
fn is_voice_id(voice: &str) -> bool {
    voice.len() == 20 && voice.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Deserialize)]
struct VoiceList {
    voices: Vec<Voice>,
}

#[derive(Deserialize)]
struct Voice {
    voice_id: String,
    name: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    labels: HashMap<String, String>,
}

#[async_trait]
impl TtsProvider for ElevenLabsTts {
    fn name(&self) -> String {
        "elevenlabs".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        let voice_id = self.voice_id(voice).await;
        debug!(
            text_len = text.len(),
            language,
            voice = %voice_id,
            model = %self.model_id,
            "Synthesizing with ElevenLabs"
        );
        let mut body = serde_json::json!({
            "text": text,
            "model_id": self.model_id,
        });
        if self.model_id.contains("v2_5") {
            body["language_code"] = language_part(language).to_ascii_lowercase().into();
        }
        let response = self
            .client
            .post(format!("{}/v1/text-to-speech/{voice_id}", self.base))
            .query(&[("output_format", format!("pcm_{RATE}"))])
            .header("xi-api-key", &self.api_key)
            .json(&body)
            .send()
            .await
            .context("Failed to reach ElevenLabs")?;
        if !response.status().is_success() {
            return Err(vendor_error("ElevenLabs", response).await);
        }
        let pcm = response
            .bytes()
            .await
            .context("Failed to read ElevenLabs' audio")?;
        pcm_to_8k_wav(&pcm, RATE)
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        // A voice speaks every language its model does.
        Ok(self
            .voices()
            .await?
            .into_iter()
            .map(|v| {
                let mut metadata = HashMap::from([
                    ("provider".to_string(), "elevenlabs".to_string()),
                    ("model".to_string(), self.model_id.clone()),
                ]);
                if let Some(category) = v.category {
                    metadata.insert("category".to_string(), category);
                }
                if let Some(accent) = v.labels.get("accent") {
                    metadata.insert("accent".to_string(), accent.clone());
                }
                VoiceInfo {
                    name: v.voice_id,
                    display_name: v.name,
                    language: "multilingual".to_string(),
                    gender: v.labels.get("gender").cloned(),
                    metadata,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_id_is_twenty_letters_and_digits() {
        assert!(is_voice_id(DEFAULT_VOICE));
        assert!(!is_voice_id("Rachel"));
        assert!(!is_voice_id("21m00Tcm4TlvDq8ikWA-"));
    }

    /// Against the real service when `FCP_TEST_ELEVENLABS_API_KEY` is set.
    #[tokio::test]
    async fn elevenlabs_speaks_when_a_key_is_set() {
        let Ok(key) = std::env::var("FCP_TEST_ELEVENLABS_API_KEY") else {
            eprintln!("FCP_TEST_ELEVENLABS_API_KEY not set; skipping");
            return;
        };
        let tts = ElevenLabsTts::new(key, None, None).unwrap();
        let wav = tts
            .synthesize("Hello from FCP.", "en-US", None)
            .await
            .unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 8000);
        assert!(reader.len() > 4000, "{} samples", reader.len());
        assert!(!tts.list_voices("en-US").await.unwrap().is_empty());
    }
}
