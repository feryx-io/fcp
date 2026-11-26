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
