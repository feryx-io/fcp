//! OpenAI TTS provider (stub implementation).
//!
//! This is a placeholder implementation. Full OpenAI TTS integration will be added later.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::Result;
use async_trait::async_trait;

/// OpenAI TTS provider (stub).
pub struct OpenAiTts {
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    model: String,
}

impl OpenAiTts {
    /// Create a new OpenAI TTS provider.
    pub fn new(api_key: String, model: String) -> Result<Self> {
        Ok(Self { api_key, model })
    }
}

#[async_trait]
impl TtsProvider for OpenAiTts {
    fn name(&self) -> String {
        "openai".to_string()
    }

    async fn synthesize(
        &self,
        _text: &str,
        _language: &str,
        _voice: Option<&str>,
    ) -> Result<Vec<u8>> {
        anyhow::bail!("OpenAI TTS provider not yet implemented")
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        anyhow::bail!("OpenAI TTS provider not yet implemented")
    }
}
