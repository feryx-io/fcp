//! Webhook delivery for presence state changes.
//!
//! Dispatches HTTP POST notifications to configured endpoints with
//! optional HMAC-SHA256 signing and exponential backoff retries.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// A registered webhook endpoint.
#[deprecated(note = "Use fcp-webhooks for general-purpose webhook delivery")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    /// Unique endpoint identifier.
    pub id: String,
    /// HTTP URL to POST events to.
    pub url: String,
    /// Optional HMAC-SHA256 secret for signing payloads.
    pub secret: Option<String>,
    /// Event type filter (e.g., `["status_changed", "*"]`). Empty = all.
    pub events: Vec<String>,
    /// Whether this endpoint is currently enabled.
    pub enabled: bool,
}

/// Payload sent to webhook endpoints.
#[derive(Debug, Clone, Serialize)]
struct WebhookPayload {
    event_type: String,
    aor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dialog_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    timestamp_epoch: u64,
}

/// Pending retry entry.
struct RetryEntry {
    endpoint_id: String,
    payload_json: String,
    signature: Option<String>,
    url: String,
    attempts: u32,
    next_attempt: Instant,
}

/// Webhook dispatcher that subscribes to the event bus and delivers
/// events to registered endpoints.
#[deprecated(note = "Use fcp-webhooks for general-purpose webhook delivery")]
pub struct WebhookDispatcher {
    endpoints: Arc<RwLock<Vec<WebhookEndpoint>>>,
    retry_queue: Arc<DashMap<String, RetryEntry>>,
    client: reqwest::Client,
    max_retries: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl WebhookDispatcher {
    /// Create a new webhook dispatcher.
    pub fn new(max_retries: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            endpoints: Arc::new(RwLock::new(Vec::new())),
            retry_queue: Arc::new(DashMap::new()),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            max_retries,
            base_delay: Duration::from_millis(base_delay_ms),
            max_delay: Duration::from_millis(max_delay_ms),
        }
    }

    /// Add or update a webhook endpoint.
    pub async fn add_endpoint(&self, endpoint: WebhookEndpoint) {
        let mut endpoints = self.endpoints.write().await;
        endpoints.retain(|e| e.id != endpoint.id);
        endpoints.push(endpoint);
    }

    /// Remove a webhook endpoint by ID.
    pub async fn remove_endpoint(&self, id: &str) -> bool {
        let mut endpoints = self.endpoints.write().await;
        let before = endpoints.len();
        endpoints.retain(|e| e.id != id);
        endpoints.len() < before
    }

    /// List all registered endpoints.
    pub async fn list_endpoints(&self) -> Vec<WebhookEndpoint> {
        self.endpoints.read().await.clone()
    }

    /// Get a specific endpoint by ID.
    pub async fn get_endpoint(&self, id: &str) -> Option<WebhookEndpoint> {
        self.endpoints
            .read()
            .await
            .iter()
            .find(|e| e.id == id)
            .cloned()
    }

    /// Compute HMAC-SHA256 signature for a payload.
    fn compute_signature(secret: &str, body: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
        mac.update(body.as_bytes());
        let result = mac.finalize();
        let bytes = result.into_bytes();
        format!("sha256={}", hex::encode(bytes))
    }

    /// Deliver a webhook payload to a specific endpoint.
    async fn deliver(&self, endpoint: &WebhookEndpoint, event_type: &str, payload_json: &str) {
        let signature = endpoint
            .secret
            .as_ref()
            .map(|s| Self::compute_signature(s, payload_json));

        let mut request = self
            .client
            .post(&endpoint.url)
            .header("Content-Type", "application/json")
            .header("X-FCP-Event", event_type);

        if let Some(ref sig) = signature {
            request = request.header("X-FCP-Signature", sig.as_str());
        }

        match request.body(payload_json.to_string()).send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!(
                    endpoint_id = %endpoint.id,
                    url = %endpoint.url,
                    event = %event_type,
                    "Webhook delivered successfully"
                );
            }
            Ok(resp) => {
                warn!(
                    endpoint_id = %endpoint.id,
                    status = %resp.status(),
                    "Webhook delivery failed, queuing retry"
                );
                self.queue_retry(
                    &endpoint.id,
                    &endpoint.url,
                    payload_json,
                    signature.as_deref(),
                );
            }
            Err(e) => {
                warn!(
                    endpoint_id = %endpoint.id,
                    error = %e,
                    "Webhook delivery error, queuing retry"
                );
                self.queue_retry(
                    &endpoint.id,
                    &endpoint.url,
                    payload_json,
                    signature.as_deref(),
                );
            }
        }
    }

    /// Add a failed delivery to the retry queue.
    fn queue_retry(
        &self,
        endpoint_id: &str,
        url: &str,
        payload_json: &str,
        signature: Option<&str>,
    ) {
        let retry_key = format!("{}-{}", endpoint_id, uuid::Uuid::new_v4());
        self.retry_queue.insert(
            retry_key,
            RetryEntry {
                endpoint_id: endpoint_id.to_string(),
                payload_json: payload_json.to_string(),
                signature: signature.map(|s| s.to_string()),
                url: url.to_string(),
                attempts: 0,
                next_attempt: Instant::now() + self.base_delay,
            },
        );
    }

    /// Process pending retries. Call this periodically.
    pub async fn process_retries(&self) {
        let now = Instant::now();
        let mut to_remove = Vec::new();

        for entry in self.retry_queue.iter() {
            if entry.value().next_attempt > now {
                continue;
            }

            let key = entry.key().clone();
            let retry = entry.value();

            if retry.attempts >= self.max_retries {
                warn!(
                    endpoint_id = %retry.endpoint_id,
                    attempts = retry.attempts,
                    "Webhook retry exhausted, dropping"
                );
                to_remove.push(key);
                continue;
            }

            let mut request = self
                .client
                .post(&retry.url)
                .header("Content-Type", "application/json");

            if let Some(ref sig) = retry.signature {
                request = request.header("X-FCP-Signature", sig.as_str());
            }

            match request.body(retry.payload_json.clone()).send().await {
                Ok(resp) if resp.status().is_success() => {
                    debug!(
                        endpoint_id = %retry.endpoint_id,
                        attempts = retry.attempts + 1,
                        "Webhook retry succeeded"
                    );
                    to_remove.push(key);
                }
                _ => {
                    // Will be retried on next cycle
                }
            }
        }

        // Remove completed/exhausted retries
        for key in &to_remove {
            self.retry_queue.remove(key);
        }

        // Bump attempt counts and compute next backoff
        for mut entry in self.retry_queue.iter_mut() {
            let retry = entry.value_mut();
            if retry.next_attempt <= now {
                retry.attempts += 1;
                let backoff = self.base_delay * 2u32.saturating_pow(retry.attempts);
                retry.next_attempt = now + backoff.min(self.max_delay);
            }
        }
    }

    /// Check if an event type matches an endpoint's filter.
    fn matches_event(endpoint: &WebhookEndpoint, event_type: &str) -> bool {
        if endpoint.events.is_empty() {
            return true;
        }
        endpoint.events.iter().any(|e| e == "*" || e == event_type)
    }
}

/// Spawn the webhook dispatcher as a background event consumer.
///
/// Subscribes to presence events from the event bus and delivers them
/// to registered webhook endpoints.
#[deprecated(note = "Use fcp-webhooks for general-purpose webhook delivery")]
pub fn spawn_webhook_consumer(
    event_bus: Arc<dyn fcp_events::EventSubscriber>,
    dispatcher: Arc<WebhookDispatcher>,
) -> tokio::task::JoinHandle<()> {
    let dispatcher_retry = Arc::clone(&dispatcher);

    // Spawn retry processor
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            dispatcher_retry.process_retries().await;
        }
    });

    // Spawn event consumer
    tokio::spawn(async move {
        let filter =
            fcp_events::EventFilter::new().types(vec![fcp_events::filter::EventType::Presence]);

        let mut stream = match event_bus.subscribe(filter).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to subscribe to presence events for webhooks");
                return;
            }
        };

        while let Some(event) = stream.next().await {
            let (event_type, payload) = match &event {
                fcp_events::PlatformEvent::Presence(pe) => match pe {
                    fcp_events::PresenceEvent::StatusChanged {
                        aor,
                        old_state,
                        new_state,
                        timestamp,
                        ..
                    } => (
                        "status_changed",
                        WebhookPayload {
                            event_type: "status_changed".to_string(),
                            aor: aor.clone(),
                            old_state: Some(old_state.clone()),
                            new_state: Some(new_state.clone()),
                            call_id: None,
                            dialog_state: None,
                            etag: None,
                            timestamp_epoch: timestamp
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        },
                    ),
                    fcp_events::PresenceEvent::DialogChanged {
                        aor,
                        call_id,
                        dialog_state,
                        timestamp,
                        ..
                    } => (
                        "dialog_changed",
                        WebhookPayload {
                            event_type: "dialog_changed".to_string(),
                            aor: aor.clone(),
                            old_state: None,
                            new_state: None,
                            call_id: Some(call_id.clone()),
                            dialog_state: Some(dialog_state.clone()),
                            etag: None,
                            timestamp_epoch: timestamp
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        },
                    ),
                    fcp_events::PresenceEvent::PublishExpired {
                        aor,
                        etag,
                        timestamp,
                        ..
                    } => (
                        "publish_expired",
                        WebhookPayload {
                            event_type: "publish_expired".to_string(),
                            aor: aor.clone(),
                            old_state: None,
                            new_state: None,
                            call_id: None,
                            dialog_state: None,
                            etag: Some(etag.clone()),
                            timestamp_epoch: timestamp
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        },
                    ),
                    _ => continue,
                },
                _ => continue,
            };

            let payload_json = match serde_json::to_string(&payload) {
                Ok(json) => json,
                Err(e) => {
                    warn!(error = %e, "Failed to serialize webhook payload");
                    continue;
                }
            };

            let endpoints = dispatcher.endpoints.read().await;
            for endpoint in endpoints.iter() {
                if !endpoint.enabled {
                    continue;
                }
                if !WebhookDispatcher::matches_event(endpoint, event_type) {
                    continue;
                }
                dispatcher
                    .deliver(endpoint, event_type, &payload_json)
                    .await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_signature() {
        let sig = WebhookDispatcher::compute_signature("my-secret", r#"{"test": true}"#);
        assert!(sig.starts_with("sha256="));
        assert_eq!(sig.len(), 7 + 64); // "sha256=" + 64 hex chars
    }

    #[test]
    fn test_event_filter_matching() {
        let all = WebhookEndpoint {
            id: "1".to_string(),
            url: "http://example.com".to_string(),
            secret: None,
            events: vec![],
            enabled: true,
        };
        assert!(WebhookDispatcher::matches_event(&all, "status_changed"));
        assert!(WebhookDispatcher::matches_event(&all, "dialog_changed"));

        let wildcard = WebhookEndpoint {
            events: vec!["*".to_string()],
            ..all.clone()
        };
        assert!(WebhookDispatcher::matches_event(&wildcard, "anything"));

        let specific = WebhookEndpoint {
            events: vec!["status_changed".to_string()],
            ..all.clone()
        };
        assert!(WebhookDispatcher::matches_event(
            &specific,
            "status_changed"
        ));
        assert!(!WebhookDispatcher::matches_event(
            &specific,
            "dialog_changed"
        ));
    }

    #[tokio::test]
    async fn test_endpoint_crud() {
        let dispatcher = WebhookDispatcher::new(3, 1000, 30000);

        let ep = WebhookEndpoint {
            id: "ep1".to_string(),
            url: "http://example.com/hook".to_string(),
            secret: Some("secret".to_string()),
            events: vec!["status_changed".to_string()],
            enabled: true,
        };

        dispatcher.add_endpoint(ep.clone()).await;
        assert_eq!(dispatcher.list_endpoints().await.len(), 1);

        let fetched = dispatcher.get_endpoint("ep1").await.unwrap();
        assert_eq!(fetched.url, "http://example.com/hook");

        assert!(dispatcher.remove_endpoint("ep1").await);
        assert_eq!(dispatcher.list_endpoints().await.len(), 0);
        assert!(!dispatcher.remove_endpoint("nonexistent").await);
    }
}
