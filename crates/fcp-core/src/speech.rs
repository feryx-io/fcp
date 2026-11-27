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

    /// The platform's providers held back from tenants
    /// (`docs/TENANT_SPEECH.md` S3): all are offered unless named here.
    async fn held_back_providers(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// What the platform charges per million characters for each of its
    /// providers (`docs/SPEECH_CAPS.md` §2): an estimated cost, not a cap.
    async fn speech_prices(&self) -> anyhow::Result<BTreeMap<String, f64>> {
        Ok(BTreeMap::new())
    }

    async fn put_speech_prices(&self, _prices: &BTreeMap<String, f64>) -> anyhow::Result<()> {
        anyhow::bail!("this store keeps no speech prices")
    }

    async fn put_held_back_providers(&self, _providers: &[String]) -> anyhow::Result<()> {
        anyhow::bail!("this store keeps no platform speech settings")
    }
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
    /// What the vendor charges per million characters, for an estimated
    /// cost beside the usage (`docs/SPEECH_CAPS.md` §2); in the tenant's
    /// own currency.
    #[serde(default)]
    pub price_per_million: Option<f64>,
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

/// What a tenant's speech cost in one hour with one provider
/// (`docs/TENANT_SPEECH.md` S4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechUsage {
    pub tenant_id: TenantId,
    /// A platform provider's name, or `account:<uuid>`.
    pub provider: String,
    /// The hour's start.
    pub hour: SystemTime,
    pub requests: u64,
    pub characters: u64,
    /// What the provider spoke; the rest came from the cache.
    pub synthesized_characters: u64,
    /// The site that counted it (`docs/TENANT_SPEECH.md` S5); none for
    /// this deployment's own.
    #[serde(default)]
    pub site_id: Option<String>,
}

/// Where speech usage is counted.
#[async_trait]
pub trait SpeechUsageStore: Send + Sync {
    /// Add these counts to what each hour holds.
    async fn add_speech_usage(&self, usage: &[SpeechUsage]) -> anyhow::Result<()>;
    /// `tenant`'s hours in `[from, to)`, oldest first.
    async fn speech_usage(
        &self,
        tenant_id: &TenantId,
        from: SystemTime,
        to: SystemTime,
    ) -> anyhow::Result<Vec<SpeechUsage>>;

    /// This deployment's own hours changed since `after` (a `ship_seq`),
    /// oldest change first: what a site's link ships up.
    async fn speech_usage_after(
        &self,
        _after: i64,
        _limit: usize,
    ) -> anyhow::Result<Vec<(i64, SpeechUsage)>> {
        Ok(Vec::new())
    }

    /// A site's hours as it counted them, each set rather than added to.
    async fn put_site_speech_usage(
        &self,
        _site_id: &str,
        _usage: &[SpeechUsage],
    ) -> anyhow::Result<()> {
        anyhow::bail!("this store keeps no sites' speech usage")
    }

    /// Every cap: the platform's defaults and every tenant's
    /// (`docs/SPEECH_CAPS.md`), for the TTS server to enforce.
    async fn all_speech_caps(&self) -> anyhow::Result<Vec<SpeechCap>> {
        Ok(Vec::new())
    }

    /// The caps on `tenant`, or with `None` the platform's defaults.
    async fn speech_caps(&self, _tenant_id: Option<&TenantId>) -> anyhow::Result<Vec<SpeechCap>> {
        Ok(Vec::new())
    }

    /// Keep a cap, replacing the one of its kind on its tenant and
    /// provider.
    async fn put_speech_cap(&self, _cap: &SpeechCap) -> anyhow::Result<()> {
        anyhow::bail!("this store keeps no speech caps")
    }

    /// Remove a cap; whether there was one.
    async fn delete_speech_cap(
        &self,
        _tenant_id: Option<&TenantId>,
        _provider: &str,
        _kind: SpeechCapKind,
    ) -> anyhow::Result<bool> {
        anyhow::bail!("this store keeps no speech caps")
    }

    /// Note that `tenant` passed `level` of its cap on `provider` in the
    /// month starting `month`; whether this is the first time (so the
    /// warning is raised once).
    async fn note_speech_cap(
        &self,
        _tenant_id: &TenantId,
        _provider: &str,
        _month: SystemTime,
        _level: CapLevel,
        _cap: u64,
        _used: u64,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    /// Every tenant's synthesized characters per provider since `from`
    /// (the month's start), this deployment's and its sites' together.
    async fn synthesized_since(
        &self,
        _from: SystemTime,
    ) -> anyhow::Result<Vec<(TenantId, String, u64)>> {
        Ok(Vec::new())
    }
}

/// How far into its cap a tenant is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapLevel {
    Ok,
    /// 80 % or more.
    Near,
    Reached,
}

impl CapLevel {
    /// Where `used` of `cap` stands.
    pub fn of(used: u64, cap: u64) -> Self {
        if used >= cap {
            Self::Reached
        } else if used.saturating_mul(5) >= cap.saturating_mul(4) {
            Self::Near
        } else {
            Self::Ok
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Near => "near",
            Self::Reached => "reached",
        }
    }
}

/// Who set a cap: the platform, which pays for its providers, or the
/// tenant, which pays for its own accounts (`docs/SPEECH_CAPS.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechCapKind {
    Platform,
    Tenant,
}

impl SpeechCapKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Tenant => "tenant",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "platform" => Some(Self::Platform),
            "tenant" => Some(Self::Tenant),
            _ => None,
        }
    }
}

/// Synthesized characters a tenant may have spoken with one provider in a
/// calendar month (UTC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechCap {
    /// None: the platform's default for every tenant.
    pub tenant_id: Option<TenantId>,
    /// A platform provider's name, or `account:<uuid>`.
    pub provider: String,
    pub kind: SpeechCapKind,
    /// None only on a tenant's platform cap: no cap for this tenant, the
    /// platform's default notwithstanding.
    pub monthly_characters: Option<u64>,
}

/// The cap that applies to `tenant` speaking with `provider`: the lowest of
/// the tenant's own and the platform's (the tenant's platform cap, else
/// the platform's default). An account is the tenant's alone to cap.
pub fn effective_cap(caps: &[SpeechCap], tenant: &TenantId, provider: &str) -> Option<u64> {
    let find = |t: Option<&TenantId>, kind| {
        caps.iter()
            .find(|c| c.tenant_id.as_ref() == t && c.provider == provider && c.kind == kind)
    };
    let own = find(Some(tenant), SpeechCapKind::Tenant).and_then(|c| c.monthly_characters);
    if account_of(provider).is_some() {
        return own;
    }
    let platform = match find(Some(tenant), SpeechCapKind::Platform) {
        Some(c) => c.monthly_characters,
        None => find(None, SpeechCapKind::Platform).and_then(|c| c.monthly_characters),
    };
    match (own, platform) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The start of the calendar month (UTC) `at` is in, and of the next.
pub fn month_of(at: SystemTime) -> (SystemTime, SystemTime) {
    use chrono::{Datelike, TimeZone, Utc};
    let t: chrono::DateTime<Utc> = at.into();
    let start = Utc
        .with_ymd_and_hms(t.year(), t.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(t);
    let (y, m) = if t.month() == 12 {
        (t.year() + 1, 1)
    } else {
        (t.year(), t.month() + 1)
    };
    let next = Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).single().unwrap_or(t);
    (start.into(), next.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(
        tenant: Option<TenantId>,
        provider: &str,
        kind: SpeechCapKind,
        n: Option<u64>,
    ) -> SpeechCap {
        SpeechCap {
            tenant_id: tenant,
            provider: provider.into(),
            kind,
            monthly_characters: n,
        }
    }

    #[test]
    fn the_lowest_cap_that_applies_is_the_cap() {
        use SpeechCapKind::*;
        let (a, b) = (TenantId::new(), TenantId::new());
        let acct = format!("{ACCOUNT_PREFIX}{}", uuid::Uuid::new_v4());
        let caps = vec![
            cap(None, "openai", Platform, Some(1000)),
            cap(Some(a), "openai", Platform, Some(5000)),
            cap(Some(a), "openai", Tenant, Some(3000)),
            cap(Some(b), "openai", Platform, None),
            cap(None, &acct, Platform, Some(1)),
            cap(Some(a), &acct, Tenant, Some(70)),
        ];
        // The tenant's platform cap overrides the default; its own is lower.
        assert_eq!(effective_cap(&caps, &a, "openai"), Some(3000));
        // None for this tenant: unlimited, the default notwithstanding.
        assert_eq!(effective_cap(&caps, &b, "openai"), None);
        // Neither: the default.
        assert_eq!(effective_cap(&caps, &TenantId::new(), "openai"), Some(1000));
        assert_eq!(effective_cap(&caps, &a, "piper"), None);
        // An account answers to its tenant alone.
        assert_eq!(effective_cap(&caps, &a, &acct), Some(70));
        assert_eq!(effective_cap(&caps, &b, &acct), None);
        // A tenant's own cap higher than the platform's changes nothing.
        let caps = vec![
            cap(None, "openai", Platform, Some(1000)),
            cap(Some(a), "openai", Tenant, Some(9000)),
        ];
        assert_eq!(effective_cap(&caps, &a, "openai"), Some(1000));
    }

    #[test]
    fn a_month_is_the_calendar_month_in_utc() {
        use chrono::{TimeZone, Utc};
        let at: SystemTime = Utc
            .with_ymd_and_hms(2026, 12, 31, 23, 59, 0)
            .unwrap()
            .into();
        let (start, next) = month_of(at);
        assert_eq!(
            start,
            SystemTime::from(Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap())
        );
        assert_eq!(
            next,
            SystemTime::from(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap())
        );
    }

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
