//! Push: ringing a sleeping phone (`docs/PUSH.md`).
//!
//! A phone app that cannot keep its connection while it sleeps registers
//! with a push address (RFC 8599): the provider that wakes it (`pn-provider`,
//! APNs or FCM), the app at that provider (`pn-param`) and the device's
//! token (`pn-prid`). [`PushConfig`] is the `[push]` section of a node that
//! registers phones or rings them: the publisher's keys, and which apps they
//! can push to.

use serde::{Deserialize, Serialize};

/// Where a sleeping app is woken: RFC 8599's `pn-provider`, `pn-param` and
/// `pn-prid`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushAddress {
    /// `apns` or `fcm`.
    pub provider: String,
    /// The app at the provider: for APNs the team and the topic
    /// (`ABCDE12345.com.thevoiceguy.anvil.voip`), for FCM the project.
    pub param: String,
    /// The device's token at the provider.
    pub prid: String,
}

impl PushAddress {
    /// The token as it may be shown or logged: its first and last characters.
    pub fn shown_prid(&self) -> String {
        shorten(&self.prid)
    }
}

/// A token's first six and last four characters, enough to tell two apart.
pub fn shorten(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 12 {
        return "…".to_string();
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

impl std::fmt::Debug for PushAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushAddress")
            .field("provider", &self.provider)
            .field("param", &self.param)
            .field("prid", &self.shown_prid())
            .finish()
    }
}

/// What a call's push says about the caller (`docs/PUSH.md` §2.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushPayload {
    /// The caller's name and number, which the phone's call screen shows
    /// before the app has done anything.
    #[default]
    Full,
    /// Only the call: the caller stays off the providers' servers.
    Minimal,
}

/// `[push]`: the publisher's keys and how long a push binding lives.
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushConfig {
    /// The longest a push binding is kept (seconds): its app re-registers
    /// when it runs, and is woken when a call needs it. Seven days unless
    /// set.
    #[serde(default)]
    pub max_expires: Option<u32>,
    #[serde(default)]
    pub payload: PushPayload,
    /// Apple's push service, one entry per team.
    #[serde(default)]
    pub apns: Vec<ApnsConfig>,
    /// Google's, one entry per project.
    #[serde(default)]
    pub fcm: Vec<FcmConfig>,
}

impl std::fmt::Debug for PushConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushConfig")
            .field("max_expires", &self.max_expires)
            .field("payload", &self.payload)
            .field("apns", &self.apns)
            .field("fcm", &self.fcm)
            .finish()
    }
}

/// One Apple team's key for VoIP pushes (token authentication).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApnsConfig {
    pub team_id: String,
    pub key_id: String,
    /// The `.p8` key: inline (sealed, or `${NAME}`) or in a file.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub key_file: Option<String>,
    /// The topics this key pushes to: the app's bundle ID with `.voip`.
    pub topics: Vec<String>,
    /// Apple's development service, for builds signed for development.
    #[serde(default)]
    pub sandbox: bool,
    /// Where to send instead of Apple's service (`https://…`; `http://`
    /// speaks HTTP/2 without TLS): a stand-in, for tests.
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl std::fmt::Debug for ApnsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApnsConfig")
            .field("team_id", &self.team_id)
            .field("key_id", &self.key_id)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .field("key_file", &self.key_file)
            .field("topics", &self.topics)
            .field("sandbox", &self.sandbox)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

/// One Firebase project's service account.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FcmConfig {
    pub project_id: String,
    /// The service account's JSON: inline (sealed, or `${NAME}`) or in a
    /// file.
    #[serde(default)]
    pub service_account: Option<String>,
    #[serde(default)]
    pub service_account_file: Option<String>,
    /// Where to send instead of Google's service: a stand-in, for tests.
    /// (The token endpoint is the service account's own `token_uri`.)
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl std::fmt::Debug for FcmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FcmConfig")
            .field("project_id", &self.project_id)
            .field(
                "service_account",
                &self.service_account.as_ref().map(|_| "<redacted>"),
            )
            .field("service_account_file", &self.service_account_file)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

/// Seven days.
pub const DEFAULT_MAX_EXPIRES: u32 = 7 * 24 * 60 * 60;

/// Why a push address is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushRefusal {
    /// A `pn-provider` without the rest (400).
    Incomplete,
    /// A provider, or an app at it, this node has no key for (555).
    Unsupported,
}

impl PushConfig {
    /// Each entry has its key, and something to push to.
    pub fn validate(&self) -> Result<(), String> {
        for apns in &self.apns {
            if apns.key.is_some() == apns.key_file.is_some() {
                return Err(format!(
                    "[[push.apns]] team {}: set one of key and key_file",
                    apns.team_id
                ));
            }
            if apns.topics.is_empty() {
                return Err(format!(
                    "[[push.apns]] team {}: topics is empty",
                    apns.team_id
                ));
            }
        }
        for fcm in &self.fcm {
            if fcm.service_account.is_some() == fcm.service_account_file.is_some() {
                return Err(format!(
                    "[[push.fcm]] project {}: set one of service_account and service_account_file",
                    fcm.project_id
                ));
            }
        }
        if self.max_expires == Some(0) {
            return Err("[push] max_expires must be above zero".into());
        }
        Ok(())
    }

    pub fn max_expires(&self) -> u32 {
        self.max_expires.unwrap_or(DEFAULT_MAX_EXPIRES)
    }

    /// The providers this node can push through, for `Feature-Caps`.
    pub fn providers(&self) -> Vec<&'static str> {
        let mut providers = Vec::new();
        if !self.apns.is_empty() {
            providers.push("apns");
        }
        if !self.fcm.is_empty() {
            providers.push("fcm");
        }
        providers
    }

    /// The APNs key for a `pn-param`: the team's ID, a dot, one of its
    /// topics.
    pub fn apns_for(&self, param: &str) -> Option<(&ApnsConfig, &str)> {
        self.apns.iter().find_map(|apns| {
            let topic = param
                .strip_prefix(apns.team_id.as_str())?
                .strip_prefix('.')?;
            apns.topics
                .iter()
                .find(|t| *t == topic)
                .map(|t| (apns, t.as_str()))
        })
    }

    /// The FCM project for a `pn-param`.
    pub fn fcm_for(&self, param: &str) -> Option<&FcmConfig> {
        self.fcm.iter().find(|f| f.project_id == param)
    }

    /// Whether this node can push to `address`.
    pub fn check(&self, address: &PushAddress) -> Result<(), PushRefusal> {
        if address.param.is_empty() || address.prid.is_empty() {
            return Err(PushRefusal::Incomplete);
        }
        let known = match address.provider.as_str() {
            "apns" => self.apns_for(&address.param).is_some(),
            "fcm" => self.fcm_for(&address.param).is_some(),
            _ => false,
        };
        if known {
            Ok(())
        } else {
            Err(PushRefusal::Unsupported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PushConfig {
        toml::from_str(
            r#"
            [[apns]]
            team_id = "ABCDE12345"
            key_id = "XYZ987"
            key = "secret"
            topics = ["com.thevoiceguy.anvil.voip"]

            [[fcm]]
            project_id = "anvil-12345"
            service_account_file = "/etc/fcp/fcm.json"
            "#,
        )
        .unwrap()
    }

    fn address(provider: &str, param: &str, prid: &str) -> PushAddress {
        PushAddress {
            provider: provider.into(),
            param: param.into(),
            prid: prid.into(),
        }
    }

    #[test]
    fn an_app_is_known_by_its_team_and_topic_or_its_project() {
        let c = config();
        assert!(c.validate().is_ok());
        assert_eq!(c.providers(), ["apns", "fcm"]);
        assert_eq!(c.max_expires(), DEFAULT_MAX_EXPIRES);
        let token = "0123456789abcdef0123";
        assert_eq!(
            c.check(&address(
                "apns",
                "ABCDE12345.com.thevoiceguy.anvil.voip",
                token
            )),
            Ok(())
        );
        assert_eq!(c.check(&address("fcm", "anvil-12345", token)), Ok(()));
        for (provider, param) in [
            ("apns", "OTHERTEAM1.com.thevoiceguy.anvil.voip"),
            ("apns", "ABCDE12345.com.thevoiceguy.anvil"),
            ("fcm", "another-project"),
            ("webpush", "anything"),
        ] {
            assert_eq!(
                c.check(&address(provider, param, token)),
                Err(PushRefusal::Unsupported),
                "{provider} {param}"
            );
        }
        assert_eq!(
            c.check(&address("fcm", "anvil-12345", "")),
            Err(PushRefusal::Incomplete)
        );
    }

    #[test]
    fn a_key_is_required_once_and_kept_out_of_logs() {
        let both: PushConfig = toml::from_str(
            r#"
            [[apns]]
            team_id = "T"
            key_id = "K"
            key = "inline"
            key_file = "/k.p8"
            topics = ["a.voip"]
            "#,
        )
        .unwrap();
        assert!(both.validate().is_err());
        let none: PushConfig = toml::from_str(
            r#"
            [[fcm]]
            project_id = "p"
            "#,
        )
        .unwrap();
        assert!(none.validate().is_err());
        let shown = format!("{:?}", config());
        assert!(!shown.contains("secret"), "{shown}");
        assert!(toml::from_str::<PushConfig>("unknown = 1").is_err());
    }

    #[test]
    fn a_token_is_shown_by_its_ends() {
        let a = address("fcm", "p", "dA1b2C3d4E5f6G7h8I9j");
        assert_eq!(a.shown_prid(), "dA1b2C…8I9j");
        assert!(!format!("{a:?}").contains("dA1b2C3d4E5f6G7h8I9j"));
        assert_eq!(shorten("short"), "…");
    }
}
