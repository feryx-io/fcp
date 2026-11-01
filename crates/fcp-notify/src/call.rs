//! The call channel: an extension is rung and told, in words
//! (`docs/E911_BLOCK_2.md` §8.1).
//!
//! The events server has no SIP stack of its own, so it asks a call
//! manager to place the call: the node's `POST /emergency/notify-call`
//! (behind its `[http] api_token`), which
//! rings the extension, speaks the notice when it is answered, and answers
//! with what became of it. A call that is not answered is a failure, so
//! the notifier's retries ring again.

use crate::NotificationChannel;
use async_trait::async_trait;
use fcp_core::e911::notify::{ContactKind, EmergencyNotice, NotifyContact};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct CallSettings {
    /// A call manager's API (`http://call-manager:8080`).
    pub call_manager_url: String,
    /// Its bearer token, when its API has auth enabled.
    pub auth_token: Option<String>,
    /// Who the call comes from, as caller ID.
    pub from: Option<String>,
    /// How long to ring before giving up.
    pub ring_timeout: Duration,
    /// How many times the notice is spoken once the call is answered.
    pub repeat: u32,
    /// How long to wait for the whole call.
    pub timeout: Duration,
}

pub struct CallChannel {
    settings: CallSettings,
    client: reqwest::Client,
}

impl CallChannel {
    pub fn new(settings: CallSettings) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(settings.timeout)
            .build()?;
        Ok(Self { settings, client })
    }

    fn url(&self) -> String {
        format!(
            "{}/emergency/notify-call",
            self.settings.call_manager_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl NotificationChannel for CallChannel {
    fn kind(&self) -> ContactKind {
        ContactKind::Call
    }

    async fn deliver(
        &self,
        contact: &NotifyContact,
        notice: &EmergencyNotice,
    ) -> anyhow::Result<()> {
        let mut request = self.client.post(self.url()).json(&serde_json::json!({
            "to": contact.display_target(),
            "from": self.settings.from,
            "text": notice.spoken(),
            "tenant_id": notice.tenant_id.to_string(),
            "call_id": notice.call_id,
            "ring_timeout_secs": self.settings.ring_timeout.as_secs(),
            "repeat": self.settings.repeat,
            "test": notice.is_test(),
            "listen": contact.listen,
        }));
        if let Some(token) = &self.settings.auth_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            let body: String = body.chars().take(200).collect();
            anyhow::bail!("the call manager answered {status}: {body}");
        }
        // The call manager answers with what became of the call; only a
        // call someone answered told anybody anything.
        let outcome: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        match outcome.get("outcome").and_then(|o| o.as_str()) {
            Some("answered") => Ok(()),
            Some(other) => anyhow::bail!("the notification call was {other}"),
            None => Ok(()),
        }
    }
}
