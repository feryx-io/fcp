//! Push: waking a sleeping phone app (`docs/PUSH.md` §2.3).
//!
//! A [`PushSender`] holds the publisher's keys from `[push]` and sends one
//! [`PushMessage`] to one [`PushAddress`]: a VoIP push through Apple's
//! service (APNs, token authentication) or a high-priority data message
//! through Google's (FCM, HTTP v1 with a service account). A push is sent
//! once, within five seconds: a call has no time for retries. What came of
//! it is a [`PushOutcome`]; a token the provider no longer knows is
//! [`PushOutcome::Gone`], and its binding should lose its push address.
//!
//! A site holds no keys: its sender relays each push up the site's link
//! (`[push.relay]`), and a cloud call manager with the keys sends it
//! (`docs/PUSH.md` §2.4). The outcome comes back the same way.

mod apns;
mod fcm;
mod jwt;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fcp_core::push::{PushAddress, PushConfig, PushPayload, RelayedPush, RelayedPushAnswer};
use serde::{Deserialize, Serialize};

pub use apns::ApnsTeam;
pub use fcm::FcmProject;

/// How long a push may take before it counts as failed.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a relayed push may take: up the link, through the cloud, to
/// the provider and back.
pub const RELAY_TIMEOUT: Duration = Duration::from_secs(10);

/// What a push is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushMessage {
    #[serde(rename = "type")]
    pub kind: PushKind,
    /// The FCP call it is about.
    pub call: String,
    /// The caller's number and name, left out under `payload = "minimal"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// As a relay carries it back.
    pub fn answer(&self) -> RelayedPushAnswer {
        RelayedPushAnswer {
            outcome: self.label().to_string(),
            reason: match self {
                Self::Sent => None,
                Self::Gone(r) | Self::Failed(r) => Some(r.clone()),
            },
        }
    }

    /// What a relay carried back.
    pub fn from_answer(answer: RelayedPushAnswer) -> Self {
        let reason = answer.reason.unwrap_or_default();
        match answer.outcome.as_str() {
            "sent" => Self::Sent,
            "gone" => Self::Gone(reason),
            _ => Self::Failed(reason),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Gone(_) => "gone",
            Self::Failed(_) => "failed",
        }
    }
}

/// The publisher's keys, ready to send; or, at a site, the link to send
/// through.
pub struct PushSender {
    apns: Vec<ApnsTeam>,
    fcm: Vec<FcmProject>,
    payload: PushPayload,
    relay: Option<Relay>,
}

/// A site's way to the keys: its link's `/push`.
struct Relay {
    url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl Relay {
    async fn send(&self, to: &PushAddress, message: &PushMessage) -> PushOutcome {
        let body = RelayedPush {
            to: to.clone(),
            message: match serde_json::to_value(message) {
                Ok(v) => v,
                Err(e) => return PushOutcome::Failed(e.to_string()),
            },
        };
        let mut request = self.http.post(&self.url).json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => return PushOutcome::Failed(format!("the link did not answer: {e}")),
        };
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let detail = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_string))
                .unwrap_or(text);
            return PushOutcome::Failed(format!("the link answered {status}: {detail}"));
        }
        match serde_json::from_str::<RelayedPushAnswer>(&text) {
            Ok(answer) => PushOutcome::from_answer(answer),
            Err(e) => PushOutcome::Failed(format!("the link's answer: {e}")),
        }
    }
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
        if let Some(relay) = &config.relay {
            return Ok(Self {
                apns: Vec::new(),
                fcm: Vec::new(),
                payload: config.payload,
                relay: Some(Relay {
                    url: relay.url.clone(),
                    token: relay.token.as_deref().map(open).transpose()?,
                    http: reqwest::Client::builder().timeout(RELAY_TIMEOUT).build()?,
                }),
            });
        }
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
            relay: None,
        })
    }

    /// Whether this sender relays through a site's link rather than
    /// holding keys: such a node cannot send for others.
    pub fn relays(&self) -> bool {
        self.relay.is_some()
    }

    /// Send one push, and count it.
    pub async fn send(&self, to: &PushAddress, message: &PushMessage) -> PushOutcome {
        let message = message.shaped(self.payload);
        if let Some(relay) = &self.relay {
            let outcome = relay.send(to, &message).await;
            log_outcome(to, &message, &outcome, true);
            PushCounters::global().note(&to.provider, message.kind, &outcome);
            return outcome;
        }
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
        log_outcome(to, &message, &outcome, false);
        PushCounters::global().note(&to.provider, message.kind, &outcome);
        outcome
    }
}

fn log_outcome(to: &PushAddress, message: &PushMessage, outcome: &PushOutcome, relayed: bool) {
    match outcome {
        PushOutcome::Sent => tracing::debug!(
            provider = %to.provider,
            token = %to.shown_prid(),
            kind = message.kind.as_str(),
            relayed,
            "Push sent"
        ),
        PushOutcome::Gone(reason) => tracing::info!(
            provider = %to.provider,
            token = %to.shown_prid(),
            %reason,
            relayed,
            "Push token gone"
        ),
        PushOutcome::Failed(reason) => tracing::warn!(
            provider = %to.provider,
            token = %to.shown_prid(),
            %reason,
            relayed,
            "Push not sent"
        ),
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
    fn an_outcome_comes_back_down_a_relay_as_it_was() {
        for outcome in [
            PushOutcome::Sent,
            PushOutcome::Gone("Unregistered".into()),
            PushOutcome::Failed("no APNs key".into()),
        ] {
            let carried: RelayedPushAnswer =
                serde_json::from_value(serde_json::to_value(outcome.answer()).unwrap()).unwrap();
            assert_eq!(PushOutcome::from_answer(carried), outcome);
        }
        // A message survives the trip to the node with the keys.
        let message = PushMessage::new(PushKind::Call, "call-1", "tenant-1")
            .caller(Some("+15125550100".into()), None);
        let back: PushMessage =
            serde_json::from_value(serde_json::to_value(&message).unwrap()).unwrap();
        assert_eq!(back.kind, PushKind::Call);
        assert_eq!(back.from.as_deref(), Some("+15125550100"));
        assert_eq!(back.tenant, "tenant-1");
    }

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
