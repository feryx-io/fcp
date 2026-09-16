//! On-site notification of emergency calls (Kari's Law,
//! `docs/E911_BLOCK_2.md` §8).
//!
//! The notifier lives here, beside the webhook dispatcher, because this is
//! the service that already consumes every platform event and already
//! retries deliveries. On `call.emergency` it builds one notice and hands
//! it to each of the tenant's contacts at once — an email, a text, a call
//! that speaks it, a webhook. Every attempt is a row in
//! `emergency_notifications`, the call's record is marked `notified` with
//! the first delivery, and a contact that runs out of attempts raises
//! `emergency.notification_failed`.

pub mod call;
pub mod email;
pub mod sms;
pub mod webhook;

use async_trait::async_trait;
use fcp_core::e911::notify::{
    ContactKind, EmergencyNotice, EmergencyNotification, NotificationStatus, NotifyContact,
};
use fcp_core::e911::{EmergencyKind, EmergencyStore};
use fcp_core::TenantId;
use fcp_events::{CallEvent, EventPublisher, PlatformEvent};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// One way of reaching a contact.
#[async_trait]
pub trait NotificationChannel: Send + Sync {
    fn kind(&self) -> ContactKind;

    /// Deliver the notice, or say why it could not be.
    async fn deliver(
        &self,
        contact: &NotifyContact,
        notice: &EmergencyNotice,
    ) -> anyhow::Result<()>;
}

/// How hard the notifier tries (§8.3).
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Attempts per contact, the first included.
    pub max_attempts: u32,
    /// The delay before the second attempt; it doubles from there.
    pub base_delay: Duration,
    /// The longest a delay grows to.
    pub max_delay: Duration,
    /// Nothing is retried longer than this after the call was placed: a
    /// notice that arrives an hour late tells nobody anything useful.
    pub give_up_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(300),
            give_up_after: Duration::from_secs(1800),
        }
    }
}

impl RetryPolicy {
    /// When the attempt after `attempt` is due, or `None` when this
    /// contact has had all it gets.
    fn next_retry(
        &self,
        attempt: i32,
        placed_at: SystemTime,
        now: SystemTime,
    ) -> Option<SystemTime> {
        if attempt as u32 >= self.max_attempts {
            return None;
        }
        let steps = (attempt.max(1) - 1).min(16) as u32;
        let delay = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(steps))
            .min(self.max_delay);
        let due = now + delay;
        let deadline = placed_at + self.give_up_after;
        (due <= deadline).then_some(due)
    }
}

/// What the notifier has done, for `/metrics` (§9).
#[derive(Debug, Default)]
pub struct NotifierMetrics {
    /// Attempts by contact kind and outcome.
    attempts: std::sync::Mutex<std::collections::BTreeMap<(String, String), u64>>,
    /// Seconds from the call being placed to its first delivery.
    first_delivery_secs: std::sync::Mutex<Vec<f64>>,
}

impl NotifierMetrics {
    pub fn record(&self, kind: ContactKind, status: NotificationStatus) {
        if let Ok(mut map) = self.attempts.lock() {
            *map.entry((kind.to_string(), status.to_string()))
                .or_insert(0) += 1;
        }
    }

    pub fn first_delivery(&self, seconds: f64) {
        if let Ok(mut seen) = self.first_delivery_secs.lock() {
            seen.push(seconds);
        }
    }

    /// Prometheus text: attempts by kind and outcome, and how long the
    /// first delivery took.
    pub fn prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "# HELP fcp_emergency_notifications_total Emergency notification attempts, by contact kind and outcome.\n\
             # TYPE fcp_emergency_notifications_total counter\n",
        );
        if let Ok(map) = self.attempts.lock() {
            for ((kind, status), n) in map.iter() {
                out.push_str(&format!(
                    "fcp_emergency_notifications_total{{kind=\"{kind}\",status=\"{status}\"}} {n}\n"
                ));
            }
        }
        if let Ok(seen) = self.first_delivery_secs.lock() {
            let count = seen.len();
            let sum: f64 = seen.iter().sum();
            out.push_str(
                "# HELP fcp_emergency_notification_first_delivery_seconds Seconds from an emergency call to the first notification delivered.\n\
                 # TYPE fcp_emergency_notification_first_delivery_seconds summary\n",
            );
            out.push_str(&format!(
                "fcp_emergency_notification_first_delivery_seconds_sum {sum}\n\
                 fcp_emergency_notification_first_delivery_seconds_count {count}\n"
            ));
        }
        out
    }
}

/// Delivers on-site notifications and keeps the record of having done so.
pub struct EmergencyNotifier {
    store: Arc<dyn EmergencyStore>,
    channels: HashMap<ContactKind, Arc<dyn NotificationChannel>>,
    events: Option<Arc<dyn EventPublisher>>,
    policy: RetryPolicy,
    metrics: Arc<NotifierMetrics>,
}

impl EmergencyNotifier {
    pub fn new(store: Arc<dyn EmergencyStore>, policy: RetryPolicy) -> Self {
        Self {
            store,
            channels: HashMap::new(),
            events: None,
            policy,
            metrics: Arc::new(NotifierMetrics::default()),
        }
    }

    /// Add a way of reaching contacts; the last one for a kind wins.
    pub fn with_channel(mut self, channel: Arc<dyn NotificationChannel>) -> Self {
        self.channels.insert(channel.kind(), channel);
        self
    }

    /// Publish `emergency.notification_failed` when a contact runs out of
    /// attempts.
    pub fn with_events(mut self, events: Arc<dyn EventPublisher>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn metrics(&self) -> Arc<NotifierMetrics> {
        Arc::clone(&self.metrics)
    }

    /// The kinds of contact this deployment can deliver to.
    pub fn kinds(&self) -> Vec<ContactKind> {
        let mut kinds: Vec<ContactKind> = self.channels.keys().copied().collect();
        kinds.sort_by_key(|k| k.to_string());
        kinds
    }

    /// Tell everyone the tenant asked to be told, if this event is an
    /// emergency call being placed.
    pub async fn on_event(&self, event: &PlatformEvent) {
        let PlatformEvent::Call(CallEvent::Emergency {
            call_id,
            tenant_id,
            from,
            dialed,
            kind,
            callback_number,
            location_source,
            location_summary,
            client_address,
            trunk_name,
            timestamp,
            ..
        }) = event
        else {
            return;
        };
        let notice = EmergencyNotice {
            tenant_id: *tenant_id,
            call_id: bare_call_id(&call_id.to_string()),
            kind: if kind == "test" {
                EmergencyKind::Test
            } else {
                EmergencyKind::Live
            },
            dialed: dialed.clone(),
            caller: from.clone(),
            caller_name: caller_name(from),
            callback_number: callback_number.clone(),
            location_summary: location_summary.clone(),
            location_source: location_source.clone(),
            client_address: client_address.clone(),
            trunk_name: trunk_name.clone(),
            placed_at: *timestamp,
        };
        self.notify(notice).await;
    }

    /// Deliver one notice to every contact the tenant can be told at.
    pub async fn notify(&self, notice: EmergencyNotice) {
        let contacts = self.contacts(&notice.tenant_id).await;
        if contacts.is_empty() {
            warn!(
                tenant_id = %notice.tenant_id,
                call_id = %notice.call_id,
                "An emergency call was placed and this tenant has nobody to tell (Kari's Law)"
            );
            return;
        }
        // The record is written off the call path, so it may not be there
        // yet; it is what `notified` is set on, so give it a moment.
        let record_id = self.wait_for_record(&notice).await;
        info!(
            tenant_id = %notice.tenant_id,
            call_id = %notice.call_id,
            contacts = contacts.len(),
            test = notice.is_test(),
            "Delivering emergency notifications"
        );
        let attempts = contacts.into_iter().map(|contact| {
            let notification = EmergencyNotification::first(
                notice.tenant_id,
                record_id,
                notice.call_id.clone(),
                contact,
                notice.clone(),
            );
            self.attempt(notification)
        });
        futures_util::future::join_all(attempts).await;
    }

    /// The contacts of this tenant that can be delivered to here: those
    /// whose kind this deployment has a channel for.
    async fn contacts(&self, tenant_id: &TenantId) -> Vec<NotifyContact> {
        let config = match self.store.get_emergency_config(tenant_id).await {
            Ok(Some(config)) => config,
            Ok(None) => return Vec::new(),
            Err(e) => {
                warn!(error = %e, tenant_id = %tenant_id, "Emergency configuration unreadable; nobody can be told");
                return Vec::new();
            }
        };
        config
            .deliverable_contacts()
            .into_iter()
            .filter(|contact| {
                let known = self.channels.contains_key(&contact.kind);
                if !known {
                    warn!(
                        tenant_id = %tenant_id,
                        contact = %contact,
                        "No channel configured for this contact kind; it cannot be told"
                    );
                }
                known
            })
            .cloned()
            .collect()
    }

    /// The emergency call's record id, waited for briefly: the node writes
    /// it off the call path, a moment after the event.
    async fn wait_for_record(&self, notice: &EmergencyNotice) -> Option<i64> {
        for wait in [0, 200, 500, 1000] {
            if wait > 0 {
                tokio::time::sleep(Duration::from_millis(wait)).await;
            }
            match self
                .store
                .get_emergency_call_by_call_id(&notice.tenant_id, &notice.call_id)
                .await
            {
                Ok(Some(record)) => return Some(record.id),
                Ok(None) => continue,
                Err(e) => {
                    debug!(error = %e, call_id = %notice.call_id, "Emergency record lookup failed");
                    return None;
                }
            }
        }
        debug!(
            call_id = %notice.call_id,
            "No emergency call record yet; the notification is kept by call id"
        );
        None
    }

    /// One attempt at one contact, with the row it leaves behind.
    async fn attempt(&self, mut notification: EmergencyNotification) {
        let kind = notification.contact.kind;
        let Some(channel) = self.channels.get(&kind) else {
            return;
        };
        let started = SystemTime::now();
        let outcome = channel
            .deliver(&notification.contact, &notification.notice)
            .await;
        let now = SystemTime::now();
        match outcome {
            Ok(()) => {
                notification.status = NotificationStatus::Delivered;
                notification.delivered_at = Some(now);
                notification.next_retry_at = None;
                info!(
                    call_id = %notification.call_id,
                    contact = %notification.contact,
                    attempt = notification.attempt,
                    "Emergency notification delivered"
                );
                if let Some(seconds) = now
                    .duration_since(notification.notice.placed_at)
                    .ok()
                    .map(|d| d.as_secs_f64())
                {
                    self.metrics.first_delivery(seconds);
                }
                if let Some(id) = notification.call_record_id {
                    if let Err(e) = self.store.mark_emergency_call_notified(id).await {
                        warn!(error = %e, "Emergency call record not marked notified");
                    }
                }
            }
            Err(e) => {
                let error = e.to_string();
                let next = self.policy.next_retry(
                    notification.attempt,
                    notification.notice.placed_at,
                    now,
                );
                notification.status = match next {
                    Some(_) => NotificationStatus::Retrying,
                    None => NotificationStatus::Failed,
                };
                notification.next_retry_at = next;
                notification.error = Some(truncate(&error, 500));
                warn!(
                    call_id = %notification.call_id,
                    contact = %notification.contact,
                    attempt = notification.attempt,
                    status = %notification.status,
                    error = %error,
                    elapsed_ms = now.duration_since(started).unwrap_or_default().as_millis(),
                    "Emergency notification failed"
                );
                if notification.status == NotificationStatus::Failed {
                    self.announce_failure(&notification, &error).await;
                }
            }
        }
        self.metrics.record(kind, notification.status);
        if let Err(e) = self
            .store
            .insert_emergency_notification(&notification)
            .await
        {
            warn!(error = %e, "Emergency notification attempt not recorded");
        }
    }

    /// Take the attempts whose retry is due and make them. Returns how
    /// many were tried.
    pub async fn run_due_retries(&self, limit: usize) -> usize {
        let due = match self.store.due_emergency_notifications(limit).await {
            Ok(due) => due,
            Err(e) => {
                warn!(error = %e, "Could not read due emergency notifications");
                return 0;
            }
        };
        let count = due.len();
        let attempts = due.into_iter().map(|row| self.attempt(row.next_attempt()));
        futures_util::future::join_all(attempts).await;
        count
    }

    async fn announce_failure(&self, notification: &EmergencyNotification, error: &str) {
        let Some(events) = &self.events else {
            return;
        };
        let event = PlatformEvent::Call(CallEvent::EmergencyNotificationFailed {
            call_id: notification
                .call_id
                .parse()
                .unwrap_or_else(|_| fcp_core::CallId::nil()),
            tenant_id: notification.tenant_id,
            contact_kind: notification.contact.kind.to_string(),
            target: notification.contact.display_target(),
            attempts: notification.attempt as u32,
            error: truncate(error, 200),
            test: notification.is_test(),
            timestamp: SystemTime::now(),
            node_id: None,
        });
        if let Err(e) = events.publish(event).await {
            warn!(error = %e, "Could not publish emergency.notification_failed");
        }
    }
}

/// Feed every event on the bus to the notifier, for as long as the server
/// runs — the webhook consumer's shape, for the same reasons.
pub fn spawn_event_consumer(
    event_bus: Arc<dyn fcp_events::EventSubscriber>,
    notifier: Arc<EmergencyNotifier>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut stream = match event_bus.subscribe(fcp_events::EventFilter::new()).await {
            Ok(stream) => stream,
            Err(e) => {
                warn!(error = %e, "Emergency notifier could not subscribe to the event bus");
                return;
            }
        };
        info!("Emergency notification consumer started");
        while let Some(event) = stream.next().await {
            notifier.on_event(&event).await;
        }
        warn!("Emergency notification event stream ended");
    })
}

/// Every attempt whose retry is due, for as long as the server runs.
pub fn spawn_retry_processor(
    notifier: Arc<EmergencyNotifier>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let tried = notifier.run_due_retries(100).await;
            if tried > 0 {
                debug!(tried, "Emergency notification retries");
            }
        }
    })
}

/// The event's call id without its `call-` prefix: the record and the CDR
/// both keep the bare uuid.
fn bare_call_id(call_id: &str) -> String {
    call_id.trim_start_matches("call-").to_string()
}

/// The user part of a SIP URI, which is the extension or username the
/// notice names.
fn caller_name(uri: &str) -> Option<String> {
    let rest = uri
        .trim()
        .trim_start_matches("sip:")
        .trim_start_matches("sips:");
    let user = rest.split('@').next()?.trim();
    (!user.is_empty() && user != rest).then(|| user.to_string())
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_caller_is_named_by_the_user_part() {
        assert_eq!(
            caller_name("sip:alice@pbx.example.com").as_deref(),
            Some("alice")
        );
        assert_eq!(caller_name("sips:2001@10.0.0.1").as_deref(), Some("2001"));
        assert_eq!(caller_name("anonymous"), None);
    }

    /// Attempts back off, and stop when the notice is too old to be worth
    /// delivering (§8.3).
    #[test]
    fn retries_back_off_and_are_given_up_on() {
        let policy = RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(60),
            give_up_after: Duration::from_secs(600),
        };
        let placed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let now = placed + Duration::from_secs(1);
        assert_eq!(
            policy.next_retry(1, placed, now),
            Some(now + Duration::from_secs(30))
        );
        assert_eq!(
            policy.next_retry(2, placed, now),
            Some(now + Duration::from_secs(60)),
            "the delay doubles, up to max_delay"
        );
        assert_eq!(policy.next_retry(3, placed, now), None, "no attempts left");
        let late = placed + Duration::from_secs(590);
        assert_eq!(
            policy.next_retry(1, placed, late),
            None,
            "not worth delivering any more"
        );
    }
}
