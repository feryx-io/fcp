//! The webhook channel: the notice through the dispatcher that is already
//! here (`docs/E911_BLOCK_2.md` §8.1).
//!
//! The contact's target is a webhook endpoint id, so a tenant points a
//! notification at an endpoint it already manages — signed, logged and
//! visible beside its other deliveries. Retries are the notifier's, not
//! the dispatcher's, so every attempt lands in one place.

use crate::NotificationChannel;
use async_trait::async_trait;
use fcp_core::e911::notify::{ContactKind, EmergencyNotice, NotifyContact};
use fcp_webhooks::dispatcher::WebhookDispatcher;
use std::str::FromStr;
use std::sync::Arc;

pub struct WebhookChannel {
    dispatcher: Arc<WebhookDispatcher>,
}

impl WebhookChannel {
    pub fn new(dispatcher: Arc<WebhookDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl NotificationChannel for WebhookChannel {
    fn kind(&self) -> ContactKind {
        ContactKind::Webhook
    }

    async fn deliver(
        &self,
        contact: &NotifyContact,
        notice: &EmergencyNotice,
    ) -> anyhow::Result<()> {
        let endpoint = fcp_core::WebhookId::from_str(contact.target.trim())
            .map_err(|e| anyhow::anyhow!("{} is not a webhook endpoint id: {e}", contact.target))?;
        self.dispatcher
            .deliver_once(
                &endpoint,
                &notice.tenant_id,
                "emergency.notification",
                &notice.payload(),
            )
            .await
    }
}
