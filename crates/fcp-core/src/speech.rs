//! A tenant's speech settings (`docs/TENANT_SPEECH.md`, phase S2): what the
//! TTS server speaks a tenant's words with when the request names nothing.

use crate::TenantId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::SystemTime;

/// What a tenant's speech sounds like, where a request does not say.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenantSpeech {
    pub tenant_id: TenantId,
    /// One of the TTS server's providers, by name; unset is its default.
    #[serde(default)]
    pub provider: Option<String>,
    /// The voice when no language below names one; unset is the provider's.
    #[serde(default)]
    pub voice: Option<String>,
    /// The language a request without one is spoken in (`en-US`).
    #[serde(default)]
    pub language: Option<String>,
    /// A voice per language (`es-ES` → `es_ES-davefx-medium`), over `voice`.
    #[serde(default)]
    pub voices: BTreeMap<String, String>,
    pub updated_at: SystemTime,
}

impl TenantSpeech {
    /// Nothing chosen: the platform's defaults.
    pub fn defaults(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            provider: None,
            voice: None,
            language: None,
            voices: BTreeMap::new(),
            updated_at: SystemTime::now(),
        }
    }

    /// The voice for `language`: its own, else the tenant's voice.
    pub fn voice_for(&self, language: &str) -> Option<&str> {
        self.voices
            .get(language)
            .or_else(|| {
                // `es` names every Spanish.
                let base = language.split(['-', '_']).next().unwrap_or(language);
                self.voices.get(base)
            })
            .map(String::as_str)
            .or(self.voice.as_deref())
    }
}

/// Where tenants' speech settings are kept.
#[async_trait]
pub trait SpeechSettingsStore: Send + Sync {
    async fn get_tenant_speech(&self, tenant_id: &TenantId)
        -> anyhow::Result<Option<TenantSpeech>>;
    async fn put_tenant_speech(&self, speech: &TenantSpeech) -> anyhow::Result<()>;
    /// Whether there were any.
    async fn delete_tenant_speech(&self, tenant_id: &TenantId) -> anyhow::Result<bool>;
}

/// A tenant's own account with a TTS vendor (bring your own key,
/// `docs/TENANT_SPEECH.md` S3): its voices are the tenant's alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechAccount {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    /// The tenant's name for it, unique in the tenant.
    pub name: String,
    pub kind: SpeechAccountKind,
    /// AWS and Azure: where the account is.
    #[serde(default)]
    pub region: Option<String>,
    /// OpenAI's model (`tts-1`), ElevenLabs' (`eleven_multilingual_v2`).
    #[serde(default)]
    pub model: Option<String>,
    /// Another endpoint for the vendor (a regional or proxied one).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// AWS: the access key id; the secret below is the secret access key.
    #[serde(default)]
    pub access_key_id: Option<String>,
    /// The API key, subscription key or secret access key: sealed at rest,
    /// written and never read back by the API.
    #[serde(default)]
    pub secret: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// The vendors a tenant may bring an account with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SpeechAccountKind {
    Openai,
    Elevenlabs,
    Azure,
    AwsPolly,
    GoogleCloud,
}

impl SpeechAccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Elevenlabs => "elevenlabs",
            Self::Azure => "azure",
            Self::AwsPolly => "aws_polly",
            Self::GoogleCloud => "google_cloud",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::Openai,
            Self::Elevenlabs,
            Self::Azure,
            Self::AwsPolly,
            Self::GoogleCloud,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }

    /// Whether `region` is required.
    pub fn needs_region(self) -> bool {
        matches!(self, Self::Azure | Self::AwsPolly)
    }
}

/// How a tenant's settings or a request name one of its accounts as the
/// provider: `account:<id>`.
pub const ACCOUNT_PREFIX: &str = "account:";

/// The account a provider name points at, if it names one.
pub fn account_of(provider: &str) -> Option<uuid::Uuid> {
    provider.strip_prefix(ACCOUNT_PREFIX)?.parse().ok()
}

/// Where tenants' own speech accounts are kept, their secrets sealed.
#[async_trait]
pub trait SpeechAccountStore: Send + Sync {
    async fn list_speech_accounts(
        &self,
        tenant_id: &TenantId,
    ) -> anyhow::Result<Vec<SpeechAccount>>;
    async fn get_speech_account(
        &self,
        tenant_id: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<Option<SpeechAccount>>;
    async fn put_speech_account(&self, account: &SpeechAccount) -> anyhow::Result<()>;
    /// Whether there was one.
    async fn delete_speech_account(
        &self,
        tenant_id: &TenantId,
        id: &uuid::Uuid,
    ) -> anyhow::Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_has_its_own_voice_else_the_tenants() {
        let mut s = TenantSpeech::defaults(TenantId::nil());
        assert_eq!(s.voice_for("en-US"), None);
        s.voice = Some("amy".into());
        s.voices.insert("es".into(), "davefx".into());
        s.voices.insert("fr-CA".into(), "lucie".into());
        assert_eq!(s.voice_for("en-US"), Some("amy"));
        assert_eq!(s.voice_for("es-ES"), Some("davefx"));
        assert_eq!(s.voice_for("fr-CA"), Some("lucie"));
        assert_eq!(s.voice_for("fr-FR"), Some("amy"));
    }
}
