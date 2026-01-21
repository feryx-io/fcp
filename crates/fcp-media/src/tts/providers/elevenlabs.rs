//! ElevenLabs TTS provider (stub implementation).
//!
//! This is a placeholder implementation. Full ElevenLabs integration will be added later.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::Result;
use async_trait::async_trait;

/// ElevenLabs TTS provider (stub).
pub struct ElevenLabsTts {
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    model_id: Option<String>,
}

impl ElevenLabsTts {
    /// Create a new ElevenLabs TTS provider.
    pub fn new(api_key: String, model_id: Option<String>) -> Result<Self> {
        Ok(Self { api_key, model_id })
    }
}

#[async_trait]
impl TtsProvider for ElevenLabsTts {
    fn name(&self) -> String {
        "elevenlabs".to_string()
    }

    async fn synthesize(
        &self,
        _text: &str,
        _language: &str,
        _voice: Option<&str>,
    ) -> Result<Vec<u8>> {
        anyhow::bail!("ElevenLabs TTS provider not yet implemented")
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        anyhow::bail!("ElevenLabs TTS provider not yet implemented")
    }
}
