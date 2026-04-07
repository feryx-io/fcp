//! OpenAI TTS provider implementation.
//!
//! Uses the OpenAI Audio API to synthesize speech from text.
//!
//! ## API Documentation
//! - <https://platform.openai.com/docs/api-reference/audio/createSpeech>
//!
//! ## Authentication
//! Requires an API key from OpenAI.
//!
//! ## Models
//! - `tts-1`: Fast, lower quality
//! - `tts-1-hd`: Slower, higher quality
//!
//! ## Voices
//! alloy, echo, fable, onyx, nova, shimmer

use super::super::{TtsProvider, VoiceInfo};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use tracing::{debug, warn};

const API_URL: &str = "https://api.openai.com/v1/audio/speech";

/// Available OpenAI TTS voices.
const VOICES: &[(&str, &str, &str)] = &[
    ("alloy", "Alloy", "neutral"),
    ("echo", "Echo", "male"),
    ("fable", "Fable", "neutral"),
    ("onyx", "Onyx", "male"),
    ("nova", "Nova", "female"),
    ("shimmer", "Shimmer", "female"),
];

/// OpenAI TTS provider.
pub struct OpenAiTts {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAiTts {
    /// Create a new OpenAI TTS provider.
    ///
    /// # Arguments
    /// * `api_key` - OpenAI API key
    /// * `model` - Model to use (`tts-1` or `tts-1-hd`)
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            api_key,
            model,
            client,
        })
    }

    /// Select the best voice, defaulting to "nova" if none specified.
    fn select_voice(voice: Option<&str>) -> &str {
        match voice {
            Some(v) if VOICES.iter().any(|(name, _, _)| *name == v) => v,
            Some(v) => {
                warn!(voice = %v, "Unknown OpenAI voice, falling back to nova");
                "nova"
            }
            None => "nova",
        }
    }
}

#[derive(Debug, Serialize)]
struct SpeechRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
    speed: f32,
}

#[async_trait]
impl TtsProvider for OpenAiTts {
    fn name(&self) -> String {
        "openai".to_string()
    }

    async fn synthesize(&self, text: &str, language: &str, voice: Option<&str>) -> Result<Vec<u8>> {
        debug!(
            text_len = text.len(),
            language = %language,
            voice = ?voice,
            model = %self.model,
            "Synthesizing with OpenAI TTS"
        );

        let selected_voice = Self::select_voice(voice);

        // OpenAI TTS doesn't support language selection natively — the voice
        // handles multiple languages automatically.  We request PCM (wav)
        // output at the default sample rate so the TtsService cache layer
        // can resample if needed.
        let request = SpeechRequest {
            model: self.model.clone(),
            input: text.to_string(),
            voice: selected_voice.to_string(),
            response_format: "wav".to_string(),
            speed: 1.0,
        };

        let response = self
            .client
            .post(API_URL)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .context("Failed to send TTS request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            anyhow::bail!("OpenAI TTS API error (status {}): {}", status, error_text);
        }

        // OpenAI returns raw audio bytes directly (not base64)
        let audio_data = response
            .bytes()
            .await
            .context("Failed to read audio response from OpenAI")?
            .to_vec();

        debug!(
            audio_size_kb = audio_data.len() / 1024,
            voice = %selected_voice,
            "OpenAI TTS synthesis complete"
        );

        Ok(audio_data)
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        // OpenAI voices are static and language-independent
        let voices = VOICES
            .iter()
            .map(|(name, display_name, gender)| {
                let mut metadata = HashMap::new();
                metadata.insert("provider".to_string(), "openai".to_string());
                metadata.insert("model".to_string(), self.model.clone());

                VoiceInfo {
                    name: name.to_string(),
                    display_name: display_name.to_string(),
                    language: "multilingual".to_string(),
                    gender: Some(gender.to_string()),
                    metadata,
                }
            })
            .collect();

        Ok(voices)
    }
}
