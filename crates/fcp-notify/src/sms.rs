//! The SMS channel: the notice as a text through an HTTP provider
//! (`docs/E911_BLOCK_2.md` §8.1).
//!
//! The contract is the thin adapter any SMS provider can be put behind:
//!
//! ```json
//! POST <url>                       Authorization: Bearer <token>
//! { "to": "+15185550143", "from": "+15185550100",
//!   "text": "[TEST] Emergency call: alice dialled 933 …",
//!   "tenant_id": "tenant-…", "call_id": "…", "test": true }
//! ```
//!
//! Anything but a 2xx is a failure, and is retried.

use crate::NotificationChannel;
use async_trait::async_trait;
use fcp_core::e911::notify::{ContactKind, EmergencyNotice, NotifyContact};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SmsSettings {
    pub url: String,
    pub token: Option<String>,
    /// The number texts come from, when the provider wants one.
    pub from: Option<String>,
    pub timeout: Duration,
}

pub struct SmsChannel {
    settings: SmsSettings,
    client: reqwest::Client,
}

impl SmsChannel {
    pub fn new(settings: SmsSettings) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(settings.timeout)
            .build()?;
        Ok(Self { settings, client })
    }
}

#[async_trait]
impl NotificationChannel for SmsChannel {
    fn kind(&self) -> ContactKind {
        ContactKind::Sms
    }

    async fn deliver(
        &self,
        contact: &NotifyContact,
        notice: &EmergencyNotice,
    ) -> anyhow::Result<()> {
        let mut request = self
            .client
            .post(&self.settings.url)
            .json(&serde_json::json!({
                "to": contact.display_target(),
                "from": self.settings.from,
                "text": notice.sms_text(),
                "tenant_id": notice.tenant_id.to_string(),
                "call_id": notice.call_id,
                "test": notice.is_test(),
            }));
        if let Some(token) = &self.settings.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(200).collect();
            anyhow::bail!("SMS provider answered {status}: {body}");
        }
        Ok(())
    }
}
