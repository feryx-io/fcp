//! Push: waking a sleeping phone app (`docs/PUSH.md` §2.3).
//!
//! A [`PushSender`] holds the publisher's keys from `[push]` and sends one
//! [`PushMessage`] to one [`PushAddress`]: a VoIP push through Apple's
//! service (APNs, token authentication) or a high-priority data message
//! through Google's (FCM, HTTP v1 with a service account). A push is sent
//! once, within five seconds: a call has no time for retries. What came of
//! it is a [`PushOutcome`]; a token the provider no longer knows is
//! [`PushOutcome::Gone`], and its binding should lose its push address.

mod apns;
mod fcm;
mod jwt;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fcp_core::push::{PushAddress, PushConfig, PushPayload};
use serde::Serialize;

pub use apns::ApnsTeam;
pub use fcm::FcmProject;

/// How long a push may take before it counts as failed.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// What a push is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PushKind {
    /// A call is ringing for the app: show it, register, take it.
    Call,
    /// A call it was woken for is over before the app came in.
    Cancel,
    /// An administrator checking the app can be woken; the app shows it
    /// and ends it at once.
    Test,
}

impl PushKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Cancel => "cancel",
            Self::Test => "test",
        }
    }
}

/// What a push says (`docs/PUSH.md` §2.3).
#[derive(Debug, Clone, Serialize)]
pub struct PushMessage {
    #[serde(rename = "type")]
    pub kind: PushKind,
    /// The FCP call it is about.
    pub call: String,
    /// The caller's number and name, left out under `payload = "minimal"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub tenant: String,
    /// When it was sent, seconds since the epoch.
    pub sent: u64,
}

impl PushMessage {
    pub fn new(kind: PushKind, call: impl Into<String>, tenant: impl Into<String>) -> Self {
        Self {
            kind,
            call: call.into(),
            from: None,
            name: None,
            tenant: tenant.into(),
            sent: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default(),
        }
    }

    pub fn caller(mut self, from: Option<String>, name: Option<String>) -> Self {
        self.from = from;
        self.name = name;
        self
    }

    /// As configured: the caller left out of a minimal payload.
    fn shaped(&self, payload: PushPayload) -> Self {
        let mut message = self.clone();
        if payload == PushPayload::Minimal {
            message.from = None;
            message.name = None;
        }
        message
    }

    /// The payload as string fields (FCM's data message takes strings).
    fn fields(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert("type".into(), self.kind.as_str().into());
        map.insert("call".into(), self.call.clone().into());
        if let Some(from) = &self.from {
            map.insert("from".into(), from.clone().into());
        }
        if let Some(name) = &self.name {
            map.insert("name".into(), name.clone().into());
        }
        map.insert("tenant".into(), self.tenant.clone().into());
        map.insert("sent".into(), self.sent.to_string().into());
        map
    }
}

/// What came of a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// The provider took it.
    Sent,
    /// The provider no longer knows the token (the app was removed, or
    /// its token changed): the binding should lose its push address.
    Gone(String),
    /// It was not sent: the provider refused it, did not answer in time,
    /// or this node has no key for the app.
    Failed(String),
}

impl PushOutcome {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Gone(_) => "gone",
            Self::Failed(_) => "failed",
        }
    }
}

/// The publisher's keys, ready to send.
pub struct PushSender {
    apns: Vec<ApnsTeam>,
    fcm: Vec<FcmProject>,
    payload: PushPayload,
}

impl PushSender {
    /// From `[push]`. `open` turns a configured secret into its value: a
    /// sealed one (`enc:v1:…`) opened, anything else as it is; a `*_file`
    /// is read here.
    pub fn new(
        config: &PushConfig,
        open: &dyn Fn(&str) -> anyhow::Result<String>,
    ) -> anyhow::Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        let http = reqwest::Client::builder().timeout(SEND_TIMEOUT).build()?;
        // HTTP/2 without TLS, for a stand-in at an `http://` endpoint.
        let h2c = reqwest::Client::builder()
            .timeout(SEND_TIMEOUT)
            .http2_prior_knowledge()
            .build()?;
        let secret = |inline: &Option<String>, file: &Option<String>| -> anyhow::Result<String> {
            match (inline, file) {
                (Some(value), _) => open(value),
                (None, Some(path)) => std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("reading {path}: {e}")),
                (None, None) => anyhow::bail!("no key"),
            }
        };
        let apns = config
            .apns
            .iter()
            .map(|team| {
                let key = secret(&team.key, &team.key_file)
                    .map_err(|e| anyhow::anyhow!("[[push.apns]] team {}: {e}", team.team_id))?;
                ApnsTeam::new(team, &key, &http, &h2c)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let fcm = config
            .fcm
            .iter()
            .map(|project| {
                let account = secret(&project.service_account, &project.service_account_file)
                    .map_err(|e| {
                        anyhow::anyhow!("[[push.fcm]] project {}: {e}", project.project_id)
                    })?;
                FcmProject::new(project, &account, &http, &h2c)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            apns,
            fcm,
            payload: config.payload,
        })
    }

    /// Send one push, and count it.
    pub async fn send(&self, to: &PushAddress, message: &PushMessage) -> PushOutcome {
        let message = message.shaped(self.payload);
        let outcome = match to.provider.as_str() {
            "apns" => match self
                .apns
                .iter()
                .find_map(|t| t.topic_for(&to.param).map(|topic| (t, topic)))
            {
                Some((team, topic)) => team.send(&topic, &to.prid, &message).await,
                None => PushOutcome::Failed(format!("no APNs key for {}", to.param)),
            },
            "fcm" => match self.fcm.iter().find(|p| p.project_id() == to.param) {
                Some(project) => project.send(&to.prid, &message).await,
                None => PushOutcome::Failed(format!("no FCM project {}", to.param)),
            },
            other => PushOutcome::Failed(format!("no provider {other}")),
        };
        match &outcome {
            PushOutcome::Sent => tracing::debug!(
                provider = %to.provider,
                token = %to.shown_prid(),
                kind = message.kind.as_str(),
                "Push sent"
            ),
            PushOutcome::Gone(reason) => tracing::info!(
                provider = %to.provider,
                token = %to.shown_prid(),
                %reason,
                "Push token gone"
            ),
            PushOutcome::Failed(reason) => tracing::warn!(
                provider = %to.provider,
                token = %to.shown_prid(),
                %reason,
                "Push not sent"
            ),
        }
        PushCounters::global().note(&to.provider, message.kind, &outcome);
        outcome
    }
}

/// Pushes by provider, kind and outcome, for a node's `/metrics`.
#[derive(Default)]
pub struct PushCounters {
    counts:
        parking_lot::Mutex<std::collections::BTreeMap<(String, &'static str, &'static str), u64>>,
}

impl PushCounters {
    pub fn global() -> &'static PushCounters {
        static GLOBAL: std::sync::OnceLock<PushCounters> = std::sync::OnceLock::new();
        GLOBAL.get_or_init(PushCounters::default)
    }

    pub fn note(&self, provider: &str, kind: PushKind, outcome: &PushOutcome) {
        *self
            .counts
            .lock()
            .entry((provider.to_string(), kind.as_str(), outcome.label()))
            .or_default() += 1;
    }

    pub fn prometheus(&self, labels: &str) -> String {
        let map = self.counts.lock();
        if map.is_empty() {
            return String::new();
        }
        let sep = if labels.is_empty() { "" } else { "," };
        let mut out = String::from(
            "# HELP fcp_push_total Pushes by provider, kind and outcome.\n\
             # TYPE fcp_push_total counter\n",
        );
        for ((provider, kind, outcome), n) in map.iter() {
            out.push_str(&format!(
                "fcp_push_total{{{labels}{sep}provider=\"{provider}\",kind=\"{kind}\",\
                 outcome=\"{outcome}\"}} {n}\n"
            ));
        }
        out
    }
}

/// A sender shared by everything that pushes on a node.
pub type SharedPushSender = Arc<PushSender>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_payload_leaves_the_caller_out() {
        let message = PushMessage::new(PushKind::Call, "call-1", "tenant-1")
            .caller(Some("1002".into()), Some("Ann Lee".into()));
        let full = message.shaped(PushPayload::Full).fields();
        assert_eq!(full["from"], "1002");
        assert_eq!(full["name"], "Ann Lee");
        assert_eq!(full["type"], "call");
        let minimal = message.shaped(PushPayload::Minimal).fields();
        assert!(!minimal.contains_key("from"));
        assert!(!minimal.contains_key("name"));
        assert_eq!(minimal["call"], "call-1");
    }

    #[test]
    fn pushes_are_counted_by_provider_kind_and_outcome() {
        let counters = PushCounters::default();
        counters.note("apns", PushKind::Call, &PushOutcome::Sent);
        counters.note("apns", PushKind::Call, &PushOutcome::Sent);
        counters.note("fcm", PushKind::Cancel, &PushOutcome::Gone("x".into()));
        let text = counters.prometheus("node=\"a\"");
        assert!(text.contains(
            "fcp_push_total{node=\"a\",provider=\"apns\",kind=\"call\",outcome=\"sent\"} 2"
        ));
        assert!(text.contains("provider=\"fcm\",kind=\"cancel\",outcome=\"gone\"} 1"));
    }
}
