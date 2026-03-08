//! Audio prompt management for IVR and system prompts.
//!
//! This module provides types and services for managing pre-recorded and
//! TTS-generated audio prompts used by the IVR, voicemail, queue, and
//! other subsystems.

use crate::{PromptId, TenantId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

/// Category of audio prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptCategory {
    System,
    Ivr,
    Voicemail,
    Queue,
    Greeting,
    Moh,
    Custom,
}

impl std::fmt::Display for PromptCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptCategory::System => write!(f, "system"),
            PromptCategory::Ivr => write!(f, "ivr"),
            PromptCategory::Voicemail => write!(f, "voicemail"),
            PromptCategory::Queue => write!(f, "queue"),
            PromptCategory::Greeting => write!(f, "greeting"),
            PromptCategory::Moh => write!(f, "moh"),
            PromptCategory::Custom => write!(f, "custom"),
        }
    }
}

impl std::str::FromStr for PromptCategory {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "system" => Ok(PromptCategory::System),
            "ivr" => Ok(PromptCategory::Ivr),
            "voicemail" => Ok(PromptCategory::Voicemail),
            "queue" => Ok(PromptCategory::Queue),
            "greeting" => Ok(PromptCategory::Greeting),
            "moh" => Ok(PromptCategory::Moh),
            "custom" => Ok(PromptCategory::Custom),
            _ => Err(anyhow::anyhow!("Unknown prompt category: {}", s)),
        }
    }
}

/// Source type for a prompt's audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptSourceType {
    File,
    Tts,
}

impl std::fmt::Display for PromptSourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromptSourceType::File => write!(f, "file"),
            PromptSourceType::Tts => write!(f, "tts"),
        }
    }
}

impl std::str::FromStr for PromptSourceType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(PromptSourceType::File),
            "tts" => Ok(PromptSourceType::Tts),
            _ => Err(anyhow::anyhow!("Unknown prompt source type: {}", s)),
        }
    }
}

/// An audio prompt record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub id: PromptId,
    pub tenant_id: TenantId,
    /// Unique name per tenant+language, e.g. "vm_enter_pin"
    pub name: String,
    pub description: Option<String>,
    pub category: PromptCategory,
    /// BCP 47 language tag, e.g. "en-US"
    pub language: String,
    pub source_type: PromptSourceType,
    /// TTS fallback text (used when no audio file exists)
    pub tts_text: Option<String>,
    /// TTS voice identifier
    pub tts_voice: Option<String>,
    /// Path to the audio file on disk
    pub audio_path: Option<PathBuf>,
    /// Audio format (e.g. "wav")
    pub audio_format: Option<String>,
    /// Duration in milliseconds
    pub duration_ms: Option<u32>,
    /// File size in bytes
    pub file_size: Option<u64>,
    /// System prompts are seeded at startup and protected from deletion
    pub is_system: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl Prompt {
    /// Create a new prompt backed by an audio file.
    pub fn new_file(
        tenant_id: TenantId,
        name: impl Into<String>,
        category: PromptCategory,
        language: impl Into<String>,
        audio_path: PathBuf,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: PromptId::new(),
            tenant_id,
            name: name.into(),
            description: None,
            category,
            language: language.into(),
            source_type: PromptSourceType::File,
            tts_text: None,
            tts_voice: None,
            audio_path: Some(audio_path),
            audio_format: Some("wav".to_string()),
            duration_ms: None,
            file_size: None,
            is_system: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new prompt backed by TTS.
    pub fn new_tts(
        tenant_id: TenantId,
        name: impl Into<String>,
        category: PromptCategory,
        language: impl Into<String>,
        tts_text: impl Into<String>,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: PromptId::new(),
            tenant_id,
            name: name.into(),
            description: None,
            category,
            language: language.into(),
            source_type: PromptSourceType::Tts,
            tts_text: Some(tts_text.into()),
            tts_voice: None,
            audio_path: None,
            audio_format: None,
            duration_ms: None,
            file_size: None,
            is_system: false,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Storage trait for prompt records.
#[async_trait::async_trait]
pub trait PromptStore: Send + Sync {
    /// Get a prompt by ID.
    async fn get_prompt(&self, id: &PromptId) -> anyhow::Result<Option<Prompt>>;

    /// Get a prompt by tenant + name (returns first matching language).
    async fn get_prompt_by_name(
        &self,
        tenant_id: &TenantId,
        name: &str,
    ) -> anyhow::Result<Option<Prompt>>;

    /// Get a prompt by tenant + name + language.
    async fn get_prompt_by_name_and_language(
        &self,
        tenant_id: &TenantId,
        name: &str,
        language: &str,
    ) -> anyhow::Result<Option<Prompt>>;

    /// List prompts for a tenant with optional filters.
    async fn list_prompts(
        &self,
        tenant_id: &TenantId,
        category: Option<PromptCategory>,
        language: Option<&str>,
    ) -> anyhow::Result<Vec<Prompt>>;

    /// Create or update a prompt (upsert on tenant_id + name + language).
    async fn put_prompt(&self, prompt: Prompt) -> anyhow::Result<()>;

    /// Delete a prompt by ID.
    async fn delete_prompt(&self, id: &PromptId) -> anyhow::Result<()>;
}

/// Service for resolving prompt names to audio file paths.
///
/// Used by the IVR engine to find pre-recorded WAV files for system prompts,
/// falling back to TTS when no audio file is available.
pub struct PromptService {
    store: Arc<dyn PromptStore>,
    tenant_id: TenantId,
    default_language: String,
}

impl PromptService {
    /// Create a new prompt service.
    pub fn new(
        store: Arc<dyn PromptStore>,
        tenant_id: TenantId,
        default_language: impl Into<String>,
    ) -> Self {
        Self {
            store,
            tenant_id,
            default_language: default_language.into(),
        }
    }

    /// Resolve a prompt name to an audio file path.
    ///
    /// 1. Try name + language match for this tenant — return audio_path if file exists
    /// 2. Try name-only match (any language) — return audio_path if file exists
    /// 3. Return None (caller should use TTS fallback)
    pub async fn resolve(
        &self,
        name: &str,
        language: Option<&str>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let lang = language.unwrap_or(&self.default_language);

        // Try exact name + language match
        if let Some(prompt) = self
            .store
            .get_prompt_by_name_and_language(&self.tenant_id, name, lang)
            .await?
        {
            if let Some(ref path) = prompt.audio_path {
                if path.exists() {
                    return Ok(Some(path.clone()));
                }
            }
        }

        // Try name-only match (any language)
        if let Some(prompt) = self.store.get_prompt_by_name(&self.tenant_id, name).await? {
            if let Some(ref path) = prompt.audio_path {
                if path.exists() {
                    return Ok(Some(path.clone()));
                }
            }
        }

        Ok(None)
    }

    /// Persist a TTS-generated WAV back to a prompt record.
    ///
    /// Called after TTS fallback succeeds — caches the result so the
    /// next `resolve()` returns the file directly without TTS.
    pub async fn cache_tts_audio(
        &self,
        name: &str,
        language: &str,
        audio_path: PathBuf,
        file_size: u64,
    ) -> anyhow::Result<()> {
        if let Some(mut prompt) = self
            .store
            .get_prompt_by_name_and_language(&self.tenant_id, name, language)
            .await?
        {
            prompt.audio_path = Some(audio_path);
            prompt.audio_format = Some("wav".to_string());
            prompt.file_size = Some(file_size);
            prompt.updated_at = SystemTime::now();
            self.store.put_prompt(prompt).await?;
        }
        Ok(())
    }

    /// Resolve a prompt by ID to an audio file path.
    ///
    /// Used when a flow Play node references a prompt via API URL
    /// (e.g. `/api/v1/prompts/prompt-<uuid>/audio`).
    pub async fn resolve_by_id(
        &self,
        id: &PromptId,
    ) -> anyhow::Result<Option<PathBuf>> {
        if let Some(prompt) = self.store.get_prompt(id).await? {
            if let Some(ref path) = prompt.audio_path {
                if path.exists() {
                    return Ok(Some(path.clone()));
                }
            }
        }
        Ok(None)
    }

    /// Get the TTS fallback text for a prompt, if available.
    pub async fn get_tts_text(
        &self,
        name: &str,
        language: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let lang = language.unwrap_or(&self.default_language);

        if let Some(prompt) = self
            .store
            .get_prompt_by_name_and_language(&self.tenant_id, name, lang)
            .await?
        {
            return Ok(prompt.tts_text);
        }

        if let Some(prompt) = self.store.get_prompt_by_name(&self.tenant_id, name).await? {
            return Ok(prompt.tts_text);
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_prompt_category_roundtrip() {
        for cat in [
            PromptCategory::System,
            PromptCategory::Ivr,
            PromptCategory::Voicemail,
            PromptCategory::Queue,
            PromptCategory::Greeting,
            PromptCategory::Moh,
            PromptCategory::Custom,
        ] {
            let s = cat.to_string();
            let parsed: PromptCategory = s.parse().unwrap();
            assert_eq!(cat, parsed);
        }
    }

    #[test]
    fn test_prompt_source_type_roundtrip() {
        for src in [PromptSourceType::File, PromptSourceType::Tts] {
            let s = src.to_string();
            let parsed: PromptSourceType = s.parse().unwrap();
            assert_eq!(src, parsed);
        }
    }

    #[test]
    fn test_new_file_prompt() {
        let tenant_id = TenantId::from_uuid(Uuid::nil());
        let prompt = Prompt::new_file(
            tenant_id,
            "vm_enter_pin",
            PromptCategory::System,
            "en-US",
            PathBuf::from("/var/fcp/audio/prompts/pin.wav"),
        );
        assert_eq!(prompt.name, "vm_enter_pin");
        assert_eq!(prompt.source_type, PromptSourceType::File);
        assert!(prompt.audio_path.is_some());
        assert!(prompt.tts_text.is_none());
        assert!(!prompt.is_system);
    }

    #[test]
    fn test_new_tts_prompt() {
        let tenant_id = TenantId::from_uuid(Uuid::nil());
        let prompt = Prompt::new_tts(
            tenant_id,
            "vm_enter_pin",
            PromptCategory::System,
            "en-US",
            "Enter your PIN followed by pound.",
        );
        assert_eq!(prompt.name, "vm_enter_pin");
        assert_eq!(prompt.source_type, PromptSourceType::Tts);
        assert!(prompt.audio_path.is_none());
        assert_eq!(
            prompt.tts_text.as_deref(),
            Some("Enter your PIN followed by pound.")
        );
    }
}
