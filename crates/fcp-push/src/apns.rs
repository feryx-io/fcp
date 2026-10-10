//! Apple's push service: a VoIP push to the device's PushKit token over
//! HTTP/2, authenticated by a provider token (an ES256 JWT from the team's
//! `.p8` key, made again every fifty minutes; Apple refuses one older than
//! an hour).

use std::time::{Duration, Instant};

use fcp_core::push::ApnsConfig;
use serde_json::json;

use crate::jwt::{now, Es256};
use crate::{PushMessage, PushOutcome};

const PRODUCTION: &str = "https://api.push.apple.com";
const SANDBOX: &str = "https://api.sandbox.push.apple.com";
const TOKEN_LIFE: Duration = Duration::from_secs(50 * 60);

/// One team's key and the topics it pushes to.
pub struct ApnsTeam {
    team_id: String,
    key_id: String,
    signer: Es256,
    topics: Vec<String>,
    base: String,
    http: reqwest::Client,
    token: parking_lot::Mutex<Option<(String, Instant)>>,
}

impl ApnsTeam {
    pub(crate) fn new(
        config: &ApnsConfig,
        key: &str,
        http: &reqwest::Client,
        h2c: &reqwest::Client,
    ) -> anyhow::Result<Self> {
        let signer = Es256::from_pem(key)
            .map_err(|e| anyhow::anyhow!("[[push.apns]] team {}: {e}", config.team_id))?;
        let base = config
            .endpoint
            .clone()
            .unwrap_or_else(|| if config.sandbox { SANDBOX } else { PRODUCTION }.to_string())
            .trim_end_matches('/')
            .to_string();
        let http = if base.starts_with("http://") {
            h2c.clone()
        } else {
            http.clone()
        };
        Ok(Self {
            team_id: config.team_id.clone(),
            key_id: config.key_id.clone(),
            signer,
            topics: config.topics.clone(),
            base,
            http,
            token: parking_lot::Mutex::new(None),
        })
    }

    /// The topic a `pn-param` names, when it is this team's: the team's ID,
    /// a dot, one of its topics.
    pub(crate) fn topic_for(&self, param: &str) -> Option<String> {
        let topic = param
            .strip_prefix(self.team_id.as_str())?
            .strip_prefix('.')?;
        self.topics
            .iter()
            .any(|t| t == topic)
            .then(|| topic.to_string())
    }

    /// The provider token, made again when it is fifty minutes old.
    fn provider_token(&self) -> anyhow::Result<String> {
        let mut held = self.token.lock();
        if let Some((token, made)) = held.as_ref() {
            if made.elapsed() < TOKEN_LIFE {
                return Ok(token.clone());
            }
        }
        let token = self.signer.sign(
            &json!({"alg": "ES256", "kid": self.key_id}),
            &json!({"iss": self.team_id, "iat": now()}),
        )?;
        *held = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    pub(crate) async fn send(
        &self,
        topic: &str,
        token: &str,
        message: &PushMessage,
    ) -> PushOutcome {
        let provider_token = match self.provider_token() {
            Ok(t) => t,
            Err(e) => return PushOutcome::Failed(format!("signing the provider token: {e}")),
        };
        let collapse: String = message.call.chars().take(64).collect();
        let sent = self
            .http
            .post(format!("{}/3/device/{token}", self.base))
            .bearer_auth(provider_token)
            .header("apns-push-type", "voip")
            .header("apns-topic", topic)
            // Now, and now or never: a call does not wait.
            .header("apns-priority", "10")
            .header("apns-expiration", "0")
            .header("apns-collapse-id", collapse)
            .json(message)
            .send()
            .await;
        let response = match sent {
            Ok(r) => r,
            Err(e) => return PushOutcome::Failed(format!("APNs did not answer: {e}")),
        };
        let status = response.status().as_u16();
        if status == 200 {
            return PushOutcome::Sent;
        }
        let reason = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|b| b["reason"].as_str().map(str::to_string))
            .unwrap_or_default();
        match (status, reason.as_str()) {
            (410, _) | (400, "BadDeviceToken") | (400, "DeviceTokenNotForTopic") => {
                PushOutcome::Gone(format!("APNs {status} {reason}"))
            }
            (403, "ExpiredProviderToken") | (403, "InvalidProviderToken") => {
                // Made again for the next push.
                *self.token.lock() = None;
                PushOutcome::Failed(format!("APNs {status} {reason}"))
            }
            _ => PushOutcome::Failed(format!("APNs {status} {reason}")),
        }
    }
}
