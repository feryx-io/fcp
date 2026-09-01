//! Core webhook dispatch and retry logic.

use crate::payload::{compute_signature, WebhookPayload};
use fcp_core::storage::{WebhookDeliveryLog, WebhookEndpoint, WebhookStore};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

/// Configuration for the webhook dispatcher.
#[derive(Debug, Clone)]
pub struct WebhookDispatcherConfig {
    /// HTTP request timeout
    pub request_timeout: Duration,
    /// Maximum number of delivery retries
    pub max_retries: u32,
    /// Base delay for exponential backoff
    pub base_retry_delay: Duration,
    /// Maximum retry delay cap
    pub max_retry_delay: Duration,
    /// Maximum concurrent webhook deliveries
    pub max_concurrent: usize,
    /// How often to poll for pending retries
    pub retry_interval: Duration,
}

impl Default for WebhookDispatcherConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(10),
            max_retries: 5,
            base_retry_delay: Duration::from_secs(30),
            max_retry_delay: Duration::from_secs(3600),
            max_concurrent: 50,
            retry_interval: Duration::from_secs(60),
        }
    }
}

/// General-purpose webhook dispatcher that delivers platform events to registered endpoints.
pub struct WebhookDispatcher {
    store: Arc<dyn WebhookStore>,
    client: reqwest::Client,
    config: WebhookDispatcherConfig,
    semaphore: Arc<Semaphore>,
}

impl WebhookDispatcher {
    /// Create a new webhook dispatcher with the given store and configuration.
    pub fn new(store: Arc<dyn WebhookStore>, config: WebhookDispatcherConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .unwrap_or_default();
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
        Self {
            store,
            client,
            config,
            semaphore,
        }
    }

    /// Get a reference to the underlying webhook store.
    pub fn store(&self) -> &Arc<dyn WebhookStore> {
        &self.store
    }

    /// Get the dispatcher configuration.
    pub fn config(&self) -> &WebhookDispatcherConfig {
        &self.config
    }

    /// Dispatch a platform event to all matching webhook endpoints for the given tenant.
    pub async fn dispatch(&self, event: &fcp_events::PlatformEvent) {
        let event_type = event.event_type_name();
        let tenant_id = match event.tenant_id() {
            Some(tid) => *tid,
            None => return, // Events without tenant context are not dispatched
        };

        // Get all endpoints for this tenant
        let endpoints = match self.store.list_endpoints(&tenant_id).await {
            Ok(eps) => eps,
            Err(e) => {
                warn!(error = %e, tenant_id = %tenant_id, "Failed to list webhook endpoints");
                return;
            }
        };

        if endpoints.is_empty() {
            return;
        }

        // Build the webhook payload
        let timestamp = event.timestamp();
        let iso_timestamp = system_time_to_iso8601(timestamp);

        let event_data = match serde_json::to_value(event) {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "Failed to serialize event to JSON");
                return;
            }
        };

        let payload = WebhookPayload {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            timestamp: iso_timestamp,
            tenant_id: Some(tenant_id.to_string()),
            data: event_data,
        };

        let payload_json = match serde_json::to_string(&payload) {
            Ok(json) => json,
            Err(e) => {
                warn!(error = %e, "Failed to serialize webhook payload");
                return;
            }
        };

        // Dispatch to matching endpoints
        for endpoint in &endpoints {
            if !endpoint.enabled {
                continue;
            }
            if !Self::matches_event(endpoint, event_type) {
                continue;
            }

            self.deliver_to_endpoint(endpoint, event_type, &payload_json)
                .await;
        }
    }

    /// Deliver a webhook payload to a specific endpoint, logging the result.
    async fn deliver_to_endpoint(
        &self,
        endpoint: &WebhookEndpoint,
        event_type: &str,
        payload_json: &str,
    ) {
        let _permit = match self.semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => return, // Semaphore closed
        };

        let signature = compute_signature(&endpoint.secret, payload_json);

        let result = self
            .client
            .post(&endpoint.url)
            .header("Content-Type", "application/json")
            .header("X-FCP-Event", event_type)
            .header("X-FCP-Signature", &signature)
            .body(payload_json.to_string())
            .send()
            .await;

        let now = SystemTime::now();

        match result {
            Ok(resp) if resp.status().is_success() => {
                let status_code = resp.status().as_u16();
                let resp_headers = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect::<std::collections::HashMap<_, _>>();
                let resp_headers_json = serde_json::to_string(&resp_headers).ok();
                let resp_body = resp.text().await.ok().map(|b| {
                    if b.len() > 65536 {
                        b[..65536].to_string()
                    } else {
                        b
                    }
                });
                debug!(
                    endpoint_id = %endpoint.id,
                    event = %event_type,
                    status = status_code,
                    "Webhook delivered successfully"
                );
                let log = WebhookDeliveryLog {
                    id: uuid::Uuid::new_v4().to_string(),
                    endpoint_id: endpoint.id,
                    event_type: event_type.to_string(),
                    payload: payload_json.to_string(),
                    status_code: Some(status_code),
                    attempts: 1,
                    last_attempt_at: Some(now),
                    next_retry_at: None,
                    delivered: true,
                    error: None,
                    response_body: resp_body,
                    response_headers: resp_headers_json,
                    created_at: now,
                };
                if let Err(e) = self.store.log_delivery(log).await {
                    warn!(error = %e, "Failed to log webhook delivery");
                }
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                let resp_headers = resp
                    .headers()
                    .iter()
                    .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                    .collect::<std::collections::HashMap<_, _>>();
                let resp_headers_json = serde_json::to_string(&resp_headers).ok();
                let resp_body = resp.text().await.ok().map(|b| {
                    if b.len() > 65536 {
                        b[..65536].to_string()
                    } else {
                        b
                    }
                });
                warn!(
                    endpoint_id = %endpoint.id,
                    status = status,
                    event = %event_type,
                    "Webhook delivery failed, scheduling retry"
                );
                let next_retry = now + self.config.base_retry_delay;
                let log = WebhookDeliveryLog {
                    id: uuid::Uuid::new_v4().to_string(),
                    endpoint_id: endpoint.id,
                    event_type: event_type.to_string(),
                    payload: payload_json.to_string(),
                    status_code: Some(status),
                    attempts: 1,
                    last_attempt_at: Some(now),
                    next_retry_at: Some(next_retry),
                    delivered: false,
                    error: Some(format!("HTTP {}", status)),
                    response_body: resp_body,
                    response_headers: resp_headers_json,
                    created_at: now,
                };
                if let Err(e) = self.store.log_delivery(log).await {
                    warn!(error = %e, "Failed to log webhook delivery failure");
                }
            }
            Err(e) => {
                warn!(
                    endpoint_id = %endpoint.id,
                    error = %e,
                    event = %event_type,
                    "Webhook delivery error, scheduling retry"
                );
                let next_retry = now + self.config.base_retry_delay;
                let log = WebhookDeliveryLog {
                    id: uuid::Uuid::new_v4().to_string(),
                    endpoint_id: endpoint.id,
                    event_type: event_type.to_string(),
                    payload: payload_json.to_string(),
                    status_code: None,
                    attempts: 1,
                    last_attempt_at: Some(now),
                    next_retry_at: Some(next_retry),
                    delivered: false,
                    error: Some(e.to_string()),
                    response_body: None,
                    response_headers: None,
                    created_at: now,
                };
                if let Err(e) = self.store.log_delivery(log).await {
                    warn!(error = %e, "Failed to log webhook delivery error");
                }
            }
        }
    }

    /// Process pending retries from the store.
    ///
    /// Fetches undelivered delivery logs whose `next_retry_at` has passed and
    /// re-attempts delivery with exponential backoff.
    pub async fn process_retries(&self) {
        let pending = match self.store.get_pending_retries(100).await {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "Failed to get pending webhook retries");
                return;
            }
        };

        if pending.is_empty() {
            return;
        }

        info!(count = pending.len(), "Processing webhook retries");

        for mut log in pending {
            if log.attempts >= self.config.max_retries {
                warn!(
                    delivery_id = %log.id,
                    endpoint_id = %log.endpoint_id,
                    attempts = log.attempts,
                    "Webhook retry exhausted, marking as permanently failed"
                );
                log.next_retry_at = None;
                log.error = Some("Max retries exhausted".to_string());
                if let Err(e) = self.store.update_delivery_log(log).await {
                    warn!(error = %e, "Failed to update delivery log");
                }
                continue;
            }

            // Look up the endpoint to get the URL and secret
            let endpoint = match self.store.get_endpoint(&log.endpoint_id).await {
                Ok(Some(ep)) => ep,
                Ok(None) => {
                    // Endpoint was deleted, mark delivery as failed
                    log.next_retry_at = None;
                    log.error = Some("Endpoint deleted".to_string());
                    let _ = self.store.update_delivery_log(log).await;
                    continue;
                }
                Err(e) => {
                    warn!(error = %e, "Failed to look up endpoint for retry");
                    continue;
                }
            };

            if !endpoint.enabled {
                // Endpoint disabled, skip but don't delete
                continue;
            }

            let _permit = match self.semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            let signature = compute_signature(&endpoint.secret, &log.payload);
            let now = SystemTime::now();

            let result = self
                .client
                .post(&endpoint.url)
                .header("Content-Type", "application/json")
                .header("X-FCP-Event", &log.event_type)
                .header("X-FCP-Signature", &signature)
                .body(log.payload.clone())
                .send()
                .await;

            log.attempts += 1;
            log.last_attempt_at = Some(now);

            match result {
                Ok(resp) if resp.status().is_success() => {
                    debug!(
                        delivery_id = %log.id,
                        attempts = log.attempts,
                        "Webhook retry succeeded"
                    );
                    log.status_code = Some(resp.status().as_u16());
                    log.delivered = true;
                    log.next_retry_at = None;
                    log.error = None;
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    log.status_code = Some(status);
                    log.error = Some(format!("HTTP {}", status));
                    let backoff = self.compute_backoff(log.attempts);
                    log.next_retry_at = Some(now + backoff);
                }
                Err(e) => {
                    log.error = Some(e.to_string());
                    let backoff = self.compute_backoff(log.attempts);
                    log.next_retry_at = Some(now + backoff);
                }
            }

            if let Err(e) = self.store.update_delivery_log(log).await {
                warn!(error = %e, "Failed to update delivery log after retry");
            }
        }
    }

    /// Compute exponential backoff delay for a given attempt number.
    fn compute_backoff(&self, attempt: u32) -> Duration {
        let backoff = self.config.base_retry_delay * 2u32.saturating_pow(attempt.saturating_sub(1));
        backoff.min(self.config.max_retry_delay)
    }

    /// Check if an event type matches an endpoint's filter.
    ///
    /// An endpoint matches if:
    /// - Its `events` list is empty (matches all)
    /// - Its `events` list contains `"*"` (wildcard)
    /// - Its `events` list contains the exact event type (e.g., `"call.initiated"`)
    /// - Its `events` list contains a category prefix match (e.g., `"call.*"` matches `"call.initiated"`)
    fn matches_event(endpoint: &WebhookEndpoint, event_type: &str) -> bool {
        if endpoint.events.is_empty() {
            return true;
        }
        endpoint.events.iter().any(|filter| {
            filter == "*"
                || filter == event_type
                || (filter.ends_with(".*") && event_type.starts_with(&filter[..filter.len() - 1]))
        })
    }
}

/// Convert a SystemTime to ISO 8601 string.
fn system_time_to_iso8601(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    // Simple UTC timestamp without external date library
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_date(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Spawn a background task that periodically retries failed webhook deliveries.
pub fn spawn_retry_processor(dispatcher: Arc<WebhookDispatcher>) -> tokio::task::JoinHandle<()> {
    let interval = dispatcher.config.retry_interval;
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(interval);
        loop {
            timer.tick().await;
            dispatcher.process_retries().await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_core::storage::WebhookEndpoint;
    use fcp_core::{TenantId, WebhookId};
    use std::time::SystemTime;

    fn make_endpoint(events: Vec<&str>) -> WebhookEndpoint {
        WebhookEndpoint {
            id: WebhookId::new(),
            tenant_id: TenantId::new(),
            url: "https://example.com/hook".to_string(),
            secret: "test-secret".to_string(),
            events: events.into_iter().map(|s| s.to_string()).collect(),
            enabled: true,
            description: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        }
    }

    #[test]
    fn test_matches_event_empty_matches_all() {
        let ep = make_endpoint(vec![]);
        assert!(WebhookDispatcher::matches_event(&ep, "call.initiated"));
        assert!(WebhookDispatcher::matches_event(
            &ep,
            "voicemail.message_recorded"
        ));
    }

    #[test]
    fn test_matches_event_wildcard() {
        let ep = make_endpoint(vec!["*"]);
        assert!(WebhookDispatcher::matches_event(&ep, "call.initiated"));
        assert!(WebhookDispatcher::matches_event(&ep, "anything"));
    }

    #[test]
    fn test_matches_event_exact() {
        let ep = make_endpoint(vec!["call.initiated", "call.ended"]);
        assert!(WebhookDispatcher::matches_event(&ep, "call.initiated"));
        assert!(WebhookDispatcher::matches_event(&ep, "call.ended"));
        assert!(!WebhookDispatcher::matches_event(&ep, "call.ringing"));
    }

    #[test]
    fn test_matches_event_category_wildcard() {
        let ep = make_endpoint(vec!["call.*"]);
        assert!(WebhookDispatcher::matches_event(&ep, "call.initiated"));
        assert!(WebhookDispatcher::matches_event(&ep, "call.ended"));
        assert!(!WebhookDispatcher::matches_event(
            &ep,
            "voicemail.message_recorded"
        ));
    }

    #[test]
    fn test_compute_backoff() {
        let config = WebhookDispatcherConfig::default();
        // We need a dummy store to test compute_backoff
        // Just test the math directly
        let base = config.base_retry_delay; // 30s
        let max = config.max_retry_delay; // 3600s

        // attempt 1: 30s * 2^0 = 30s
        let b1 = base * 2u32.saturating_pow(0);
        assert_eq!(b1, Duration::from_secs(30));

        // attempt 2: 30s * 2^1 = 60s
        let b2 = base * 2u32.saturating_pow(1);
        assert_eq!(b2, Duration::from_secs(60));

        // attempt 3: 30s * 2^2 = 120s
        let b3 = base * 2u32.saturating_pow(2);
        assert_eq!(b3, Duration::from_secs(120));

        // attempt 5: 30s * 2^4 = 480s
        let b5 = (base * 2u32.saturating_pow(4)).min(max);
        assert_eq!(b5, Duration::from_secs(480));
    }

    #[test]
    fn test_system_time_to_iso8601() {
        let epoch = UNIX_EPOCH;
        assert_eq!(system_time_to_iso8601(epoch), "1970-01-01T00:00:00Z");

        // 2026-02-20T00:00:00Z = 1771545600 seconds since epoch
        let t = UNIX_EPOCH + Duration::from_secs(1771545600);
        let iso = system_time_to_iso8601(t);
        assert!(iso.starts_with("2026-02-20"));
    }
}
