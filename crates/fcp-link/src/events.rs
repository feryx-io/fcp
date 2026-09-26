//! The site's events, forwarded to the cloud (`docs/HYBRID_CONTROL_PLANE.md`
//! §3.3): read from the site's own event stream as one more consumer,
//! sent up the link under the node that published each, and fired in the
//! cloud once, as the tenant's, with the site named. `cdr.written` stays
//! behind: the cloud announces its own once it has stored the record.

use anyhow::{Context, Result};
use fcp_core::{ShippedAnswer, ShippedEvents, TenantId};
use fcp_events::{CdrEvent, PlatformEvent};
use fcp_storage_redis::event_bus::RedisStreamConsumer;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tracing::{debug, warn};

/// The consumer group the link reads the site's stream with.
pub const CONSUMER_GROUP: &str = "link";

pub struct EventForwarder {
    consumer: RedisStreamConsumer,
    client: reqwest::Client,
    link_url: String,
    secret: String,
    tenant: TenantId,
    forwarded: AtomicU64,
    error: Mutex<Option<String>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EventsStatus {
    pub forwarded: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl EventForwarder {
    pub fn new(
        consumer: RedisStreamConsumer,
        client: reqwest::Client,
        link_url: &str,
        secret: String,
        tenant: TenantId,
    ) -> Self {
        Self {
            consumer,
            client,
            link_url: link_url.trim_end_matches('/').to_string(),
            secret,
            tenant,
            forwarded: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }

    pub fn status(&self) -> EventsStatus {
        EventsStatus {
            forwarded: self.forwarded.load(Ordering::Relaxed),
            error: self.error.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    pub async fn run(&self) {
        if let Err(e) = self.consumer.ensure_group().await {
            warn!(error = %e, "The site's event stream cannot be read; its events are not forwarded");
            return;
        }
        loop {
            match self.step().await {
                Ok(()) => {
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                Err(e) => {
                    let text = format!("{e:#}");
                    warn!(error = %text, "Forwarding the site's events failed; trying again");
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// One read of the stream, forwarded and acknowledged as a whole:
    /// entries left unacknowledged by a failure are claimed back first.
    async fn step(&self) -> Result<()> {
        let mut entries = self
            .consumer
            .claim_stale(Duration::from_secs(30), 100)
            .await
            .context("claiming the site's unforwarded events")?;
        if entries.is_empty() {
            entries = self
                .consumer
                .read_entries(100, 2000)
                .await
                .context("reading the site's events")?;
        }
        if entries.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let mut by_node: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
        for entry in entries {
            let envelope = entry.envelope;
            if envelope.relayed {
                continue;
            }
            if envelope.event.tenant_id() != Some(&self.tenant) {
                continue;
            }
            if matches!(envelope.event, PlatformEvent::Cdr(CdrEvent::Written { .. })) {
                continue;
            }
            by_node
                .entry(envelope.node_id)
                .or_default()
                .push(serde_json::to_value(&envelope.event)?);
        }
        let batches: Vec<ShippedEvents> = by_node
            .into_iter()
            .map(|(node_id, events)| ShippedEvents { node_id, events })
            .collect();
        if !batches.is_empty() {
            let response = self
                .client
                .post(format!("{}/link/v1/events", self.link_url))
                .bearer_auth(&self.secret)
                .json(&batches)
                .send()
                .await
                .context("reaching the cloud with the site's events")?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("the cloud refused the site's events: {status} {body}");
            }
            let answer: ShippedAnswer = response
                .json()
                .await
                .context("reading the cloud's answer to the events")?;
            self.forwarded
                .fetch_add(answer.applied as u64, Ordering::Relaxed);
            debug!(
                forwarded = answer.applied,
                dropped = answer.skipped,
                "Events forwarded"
            );
        }
        self.consumer
            .ack(&ids)
            .await
            .context("acknowledging the site's events")?;
        Ok(())
    }
}
