//! Azure AI Speech text to speech (#339).
//!
//! - `POST /cognitiveservices/v1` on `<region>.tts.speech.microsoft.com`,
//!   SSML in, the key in `Ocp-Apim-Subscription-Key`, and
//!   `X-Microsoft-OutputFormat: riff-8khz-16bit-mono-pcm`: a WAV at 8 kHz.
//! - `GET /cognitiveservices/voices/list`: the region's voices.
//!
//! <https://learn.microsoft.com/azure/ai-services/speech-service/rest-text-to-speech>
//!
//! A request naming no voice gets the first neural voice of its language
//! from the list, remembered per language.

use super::super::{TtsProvider, VoiceInfo};
use super::{vendor_error, wav_to_8k};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const OUTPUT_FORMAT: &str = "riff-8khz-16bit-mono-pcm";
/// When the list has nothing for a language.
const FALLBACK_VOICE: &str = "en-US-AvaMultilingualNeural";

pub struct AzureTts {
    subscription_key: String,
    base: String,
    client: reqwest::Client,
    /// The voice chosen for each language a request named none for.
    defaults: Mutex<HashMap<String, String>>,
}

impl AzureTts {
    pub fn new(subscription_key: String, region: String, endpoint: Option<String>) -> Result<Self> {
        if subscription_key.trim().is_empty() {
            bail!("Azure TTS provider requires a non-empty subscription_key");
        }
        let region = region.trim();
        if endpoint.is_none()
            && (region.is_empty() || !region.chars().all(|c| c.is_ascii_alphanumeric()))
        {
            bail!("Azure TTS provider requires a region such as westeurope, not {region:?}");
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("fcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("Failed to create HTTP client")?;
        Ok(Self {
            subscription_key,
            base: endpoint
                .unwrap_or_else(|| format!("https://{region}.tts.speech.microsoft.com"))
                .trim_end_matches('/')
                .to_string(),
            client,
            defaults: Mutex::new(HashMap::new()),
        })
    }

    async fn voices(&self) -> Result<Vec<Voice>> {
        let response = self
            .client
            .get(format!("{}/cognitiveservices/voices/list", self.base))
            .header("Ocp-Apim-Subscription-Key", &self.subscription_key)
            .send()
            .await
            .context("Failed to reach Azure Speech")?;
        if !response.status().is_success() {
            return Err(vendor_error("Azure Speech", response).await);
        }
        response.json().await.context("Azure Speech's voice list")
    }

    /// The voice for `language` when a request names none.
    async fn default_voice(&self, language: &str) -> String {
        let key = language.to_ascii_lowercase();
        if let Some(voice) = self.defaults.lock().await.get(&key) {
            return voice.clone();
        }
        let chosen = match self.voices().await {
            Ok(voices) => voices
                .iter()
                .filter(|v| v.locale.eq_ignore_ascii_case(language))
                .find(|v| v.voice_type.as_deref() == Some("Neural"))
                .or_else(|| {
                    voices
                        .iter()
                        .find(|v| v.locale.eq_ignore_ascii_case(language))
                })
                .map(|v| v.short_name.clone()),
            Err(e) => {
                warn!(language, error = %e, "Could not list Azure voices");
                return FALLBACK_VOICE.to_string();
            }
        };
        let voice = chosen.unwrap_or_else(|| {
            warn!(
                language,
                "No Azure voice for the language; using {FALLBACK_VOICE}"
            );
            FALLBACK_VOICE.to_string()
        });
        self.defaults.lock().await.insert(key, voice.clone());
        voice
    }
}

#[derive(Deserialize)]
struct Voice {
    #[serde(rename = "ShortName")]
    short_name: String,
    #[serde(rename = "DisplayName", default)]
    display_name: Option<String>,
    #[serde(rename = "Locale")]
    locale: String,
    #[serde(rename = "Gender", default)]
    gender: Option<String>,
    #[serde(rename = "VoiceType", default)]
    voice_type: Option<String>,
}

/// The SSML document Azure is sent.
fn ssml(text: &str, language: &str, voice: &str) -> String {
    format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"{}\">\
         <voice name=\"{}\">{}</voice></speak>",
        escape(language),
        escape(voice),
        escape(text)
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[async_trait]
impl TtsProvider for AzureTts {
    fn name(&self) -> String {
        "azure".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        let voice = match voice.filter(|v| !v.trim().is_empty()) {
            Some(v) => v.to_string(),
            None => self.default_voice(language).await,
        };
        debug!(text_len = text.len(), language, voice = %voice, "Synthesizing with Azure");
        let response = self
            .client
            .post(format!("{}/cognitiveservices/v1", self.base))
            .header("Ocp-Apim-Subscription-Key", &self.subscription_key)
            .header("Content-Type", "application/ssml+xml")
            .header("X-Microsoft-OutputFormat", OUTPUT_FORMAT)
            .body(ssml(text, language, &voice))
            .send()
            .await
            .context("Failed to reach Azure Speech")?;
        if !response.status().is_success() {
            return Err(vendor_error("Azure Speech", response).await);
        }
        let wav = response
            .bytes()
            .await
            .context("Failed to read Azure's audio")?;
        wav_to_8k(&wav).context("Azure's audio")
    }

    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        Ok(self
            .voices()
            .await?
            .into_iter()
            .filter(|v| {
                language.is_empty()
                    || v.locale.eq_ignore_ascii_case(language)
                    || (!language.contains('-')
                        && super::language_part(&v.locale).eq_ignore_ascii_case(language))
            })
            .map(|v| VoiceInfo {
                display_name: v.display_name.unwrap_or_else(|| v.short_name.clone()),
                name: v.short_name,
                language: v.locale,
                gender: v.gender.map(|g| g.to_ascii_lowercase()),
                metadata: HashMap::from([
                    ("provider".to_string(), "azure".to_string()),
                    ("voice_type".to_string(), v.voice_type.unwrap_or_default()),
                ]),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_escaped_into_the_ssml() {
        let doc = ssml("Tom & Jerry's <menu>", "en-GB", "en-GB-SoniaNeural");
        assert_eq!(
            doc,
            "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"en-GB\">\
             <voice name=\"en-GB-SoniaNeural\">Tom &amp; Jerry&apos;s &lt;menu&gt;</voice></speak>"
        );
    }

    #[test]
    fn a_region_is_required_without_an_endpoint() {
        assert!(AzureTts::new("key".into(), "".into(), None).is_err());
        assert!(AzureTts::new("key".into(), "evil.example/".into(), None).is_err());
        assert!(AzureTts::new("key".into(), "westeurope".into(), None).is_ok());
        assert!(AzureTts::new("key".into(), "".into(), Some("http://127.0.0.1:9".into())).is_ok());
    }

    /// Against the real service when `FCP_TEST_AZURE_SPEECH_KEY` and
    /// `FCP_TEST_AZURE_SPEECH_REGION` are set.
    #[tokio::test]
    async fn azure_speaks_when_a_key_is_set() {
        let (Ok(key), Ok(region)) = (
            std::env::var("FCP_TEST_AZURE_SPEECH_KEY"),
            std::env::var("FCP_TEST_AZURE_SPEECH_REGION"),
        ) else {
            eprintln!("FCP_TEST_AZURE_SPEECH_KEY/REGION not set; skipping");
            return;
        };
        let tts = AzureTts::new(key, region, None).unwrap();
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
