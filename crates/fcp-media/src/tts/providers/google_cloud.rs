//! Google Cloud Text-to-Speech provider implementation.
//!
//! Uses the Google Cloud Text-to-Speech API (v1) to synthesize speech from text.
//!
//! ## API Documentation
//! - REST API: https://cloud.google.com/text-to-speech/docs/reference/rest
//! - Supported voices: https://cloud.google.com/text-to-speech/docs/voices
//!
//! ## Authentication
//! Requires an API key from Google Cloud Console with Text-to-Speech API enabled.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, warn};

const API_BASE: &str = "https://texttospeech.googleapis.com/v1";

/// Google Cloud Text-to-Speech provider.
pub struct GoogleCloudTts {
    api_key: String,
    client: reqwest::Client,
}

impl GoogleCloudTts {
    /// Create a new Google Cloud TTS provider.
    pub fn new(api_key: String) -> Result<Self> {
        if api_key.trim().is_empty() {
            bail!("Google Cloud TTS provider requires a non-empty API key");
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { api_key, client })
    }

    /// Select best voice for language.
    fn select_voice(&self, language: &str, voice_name: Option<&str>) -> String {
        if let Some(name) = voice_name {
            return name.to_string();
        }

        // Default voices for common languages (Neural2 variants)
        match language {
            "en-US" => "en-US-Neural2-C",
            "en-GB" => "en-GB-Neural2-A",
            "es-ES" => "es-ES-Neural2-A",
            "es-US" => "es-US-Neural2-A",
            "fr-FR" => "fr-FR-Neural2-A",
            "de-DE" => "de-DE-Neural2-A",
            "it-IT" => "it-IT-Neural2-A",
            "pt-BR" => "pt-BR-Neural2-A",
            "ja-JP" => "ja-JP-Neural2-B",
            "ko-KR" => "ko-KR-Neural2-A",
            "zh-CN" => "cmn-CN-Wavenet-A",
            _ => {
                warn!(language = %language, "Unknown language, using en-US default");
                "en-US-Neural2-C"
            }
        }
        .to_string()
    }
}

#[async_trait]
impl TtsProvider for GoogleCloudTts {
    fn name(&self) -> String {
        "google_cloud".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        debug!(
            text_len = text.len(),
            language = %language,
            voice = ?voice,
            "Synthesizing with Google Cloud TTS"
        );

        let voice_name = self.select_voice(language, voice);

        let request = SynthesizeRequest {
            input: TextInput {
                text: text.to_string(),
            },
            voice: VoiceSelection {
                language_code: language.to_string(),
                name: voice_name.clone(),
            },
            audio_config: AudioConfig {
                audio_encoding: "LINEAR16".to_string(),
                sample_rate_hertz: 8000,
            },
        };

        let url = format!("{}/text:synthesize?key={}", API_BASE, self.api_key);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send TTS request to Google Cloud")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!(
                "Google Cloud TTS API error (status {}): {}",
                status,
                error_text
            );
        }

        let synthesis_response: SynthesizeResponse = response
            .json()
            .await
            .context("Failed to parse TTS response from Google Cloud")?;

        // Decode base64 audio content
        let audio_data = general_purpose::STANDARD
            .decode(&synthesis_response.audio_content)
            .context("Failed to decode base64 audio content")?;

        debug!(
            audio_size_kb = audio_data.len() / 1024,
            voice = %voice_name,
            "Google Cloud TTS synthesis complete"
        );

        Ok(audio_data)
    }

    async fn list_voices(&self, language: &str) -> Result<Vec<VoiceInfo>> {
        debug!(language = %language, "Listing Google Cloud TTS voices");

        let url = format!("{}/voices?key={}", API_BASE, self.api_key);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to list voices from Google Cloud")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!(
                "Google Cloud TTS API error (status {}): {}",
                status,
                error_text
            );
        }

        let list_response: ListVoicesResponse = response
            .json()
            .await
            .context("Failed to parse voices response from Google Cloud")?;

        // Filter voices by language and convert to VoiceInfo
        let voices: Vec<VoiceInfo> = list_response
            .voices
            .into_iter()
            .filter(|v| {
                v.language_codes
                    .iter()
                    .any(|lang| lang.starts_with(language))
            })
            .map(|v| {
                let mut metadata = HashMap::new();
                metadata.insert(
                    "ssml_gender".to_string(),
                    v.ssml_gender
                        .clone()
                        .unwrap_or_else(|| "NEUTRAL".to_string()),
                );
                metadata.insert(
                    "natural_sample_rate".to_string(),
                    v.natural_sample_rate_hertz.to_string(),
                );

                VoiceInfo {
                    name: v.name.clone(),
                    display_name: v.name.clone(),
                    language: v.language_codes.first().cloned().unwrap_or_default(),
                    gender: v.ssml_gender.clone().map(|g| match g.as_str() {
                        "MALE" => "male".to_string(),
                        "FEMALE" => "female".to_string(),
                        _ => "neutral".to_string(),
                    }),
                    metadata,
                }
            })
            .collect();

        debug!(voice_count = voices.len(), "Listed Google Cloud TTS voices");

        Ok(voices)
    }
}

// Request/Response types for Google Cloud TTS API

#[derive(Debug, Serialize)]
struct SynthesizeRequest {
    input: TextInput,
    voice: VoiceSelection,
    #[serde(rename = "audioConfig")]
    audio_config: AudioConfig,
}

#[derive(Debug, Serialize)]
struct TextInput {
    text: String,
}

#[derive(Debug, Serialize)]
struct VoiceSelection {
    #[serde(rename = "languageCode")]
    language_code: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct AudioConfig {
    #[serde(rename = "audioEncoding")]
    audio_encoding: String,
    #[serde(rename = "sampleRateHertz")]
    sample_rate_hertz: u32,
}

#[derive(Debug, Deserialize)]
struct SynthesizeResponse {
    #[serde(rename = "audioContent")]
    audio_content: String,
}

#[derive(Debug, Deserialize)]
struct ListVoicesResponse {
    voices: Vec<GoogleVoice>,
}

#[derive(Debug, Deserialize)]
struct GoogleVoice {
    #[serde(rename = "languageCodes")]
    language_codes: Vec<String>,
    name: String,
    #[serde(rename = "ssmlGender")]
    ssml_gender: Option<String>,
    #[serde(rename = "naturalSampleRateHertz")]
    natural_sample_rate_hertz: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_selection() {
        let provider = GoogleCloudTts::new("test-key".to_string()).unwrap();

        assert_eq!(provider.select_voice("en-US", None), "en-US-Neural2-C");
        assert_eq!(provider.select_voice("es-ES", None), "es-ES-Neural2-A");
        assert_eq!(
            provider.select_voice("en-US", Some("en-US-Wavenet-D")),
            "en-US-Wavenet-D"
        );
    }
}
