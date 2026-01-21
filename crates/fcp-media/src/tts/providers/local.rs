//! Local TTS provider (stub implementation).
//!
//! This is a placeholder for local TTS engines like espeak, festival, and piper.
//! Full implementation will be added later.

use super::super::{TtsProvider, VoiceInfo};
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;

/// Local TTS provider (stub).
pub struct LocalTts {
    #[allow(dead_code)]
    engine: String,
    #[allow(dead_code)]
    executable_path: Option<PathBuf>,
}

impl LocalTts {
    /// Create a new local TTS provider.
    pub fn new(engine: String, executable_path: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            engine,
            executable_path,
        })
    }
}

#[async_trait]
impl TtsProvider for LocalTts {
    fn name(&self) -> String {
        format!("local_{}", self.engine)
    }

    async fn synthesize(
        &self,
        _text: &str,
        _language: &str,
        _voice: Option<&str>,
    ) -> Result<Vec<u8>> {
        anyhow::bail!("Local TTS provider not yet implemented")
    }

    async fn list_voices(&self, _language: &str) -> Result<Vec<VoiceInfo>> {
        anyhow::bail!("Local TTS provider not yet implemented")
    }
}
