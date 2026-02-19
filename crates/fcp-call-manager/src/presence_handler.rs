//! SIP Presence handler — SUBSCRIBE/PUBLISH/NOTIFY for presence and BLF.
//!
//! Implements the `PresenceHandler` trait from fcp-sip, wiring SIP signaling
//! to the `PresenceManager` from fcp-presence.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use sip_core::SipUri;
use tracing::{debug, info, warn};

use fcp_core::TenantId;
use fcp_presence::{PresenceManager, Watcher};
use fcp_sip::presence::PresenceHandler;
use fcp_sip::uac::SipUserAgentClient;

/// SIP presence handler that bridges SIP signaling with PresenceManager.
pub struct SipPresenceHandler {
    /// Presence state manager
    presence: PresenceManager,
    /// UAC for sending NOTIFY requests
    uac: Arc<SipUserAgentClient>,
    /// Default tenant (single-tenant deployment)
    tenant_id: TenantId,
    /// Default subscription expiry in seconds
    default_expires: u32,
    /// Rate limiter for SUBSCRIBE requests (keyed by source IP and AOR)
    subscribe_limiter: Arc<sip_ratelimit::RateLimiter>,
    /// Rate limiter for PUBLISH requests (keyed by source IP and AOR)
    publish_limiter: Arc<sip_ratelimit::RateLimiter>,
}

/// Parsed SUBSCRIBE context for easier dispatch.
struct SubscribeContext {
    event: String,
    presentity_aor: String,
    watcher_aor: String,
    contact_uri: String,
    expires: u32,
}

impl SipPresenceHandler {
    pub fn new(
        presence: PresenceManager,
        uac: Arc<SipUserAgentClient>,
        tenant_id: TenantId,
        default_expires: u32,
    ) -> Self {
        let config = presence.config();
        let subscribe_limiter = Arc::new(sip_ratelimit::RateLimiter::new(
            sip_ratelimit::RateLimitConfig::new(config.subscribe_rate_limit_per_ip, 60)
                .unwrap_or_else(|_| sip_ratelimit::RateLimitConfig::new(20, 60).unwrap()),
        ));
        let publish_limiter = Arc::new(sip_ratelimit::RateLimiter::new(
            sip_ratelimit::RateLimitConfig::new(config.publish_rate_limit_per_ip, 60)
                .unwrap_or_else(|_| sip_ratelimit::RateLimitConfig::new(30, 60).unwrap()),
        ));
        Self {
            presence,
            uac,
            tenant_id,
            default_expires,
            subscribe_limiter,
            publish_limiter,
        }
    }

    /// Extract the presentity AOR from a SIP Request-URI.
    fn extract_presentity_aor(request: &sip_core::Request) -> Option<String> {
        // Request-URI is like sip:alice@example.com
        let uri_str = request.uri().to_string();
        let uri = uri_str
            .strip_prefix("sip:")
            .or_else(|| uri_str.strip_prefix("sips:"))
            .unwrap_or(&uri_str);

        if uri.contains('@') {
            // Strip port/params: alice@example.com:5060;transport=tcp → alice@example.com
            let aor = uri.split(';').next().unwrap_or(uri);
            if let Some((user_host, _port)) = aor.rsplit_once(':') {
                if user_host.contains('@') {
                    return Some(user_host.to_string());
                }
            }
            Some(aor.to_string())
        } else {
            None
        }
    }

    /// Extract the watcher AOR from the From header.
    fn extract_watcher_aor(request: &sip_core::Request) -> Option<String> {
        let from = request.headers().get("From")?;
        crate::extract_aor(&*from)
    }

    /// Extract the Contact URI from the request (where to send NOTIFY).
    fn extract_contact_uri(request: &sip_core::Request) -> Option<String> {
        let contact = request.headers().get("Contact")?;
        let contact_str: &str = &*contact;

        // Extract URI from < > brackets if present
        if let Some(start) = contact_str.find('<') {
            let end = contact_str.find('>').unwrap_or(contact_str.len());
            Some(contact_str[start + 1..end].to_string())
        } else {
            // Bare URI
            Some(contact_str.trim().to_string())
        }
    }

    /// Extract the Event header value.
    fn extract_event(request: &sip_core::Request) -> Option<String> {
        let event = sip_parse::header(request.headers(), "Event")?;
        // Event header may have parameters: "presence;id=xxx" — take just the package
        Some(event.split(';').next().unwrap_or(&event).trim().to_lowercase())
    }

    /// Extract the Expires header value.
    fn extract_expires(request: &sip_core::Request) -> Option<u32> {
        sip_parse::header(request.headers(), "Expires")
            .and_then(|v| v.parse::<u32>().ok())
    }

    /// Extract SIP-If-Match header value.
    fn extract_sip_if_match(request: &sip_core::Request) -> Option<String> {
        sip_parse::header(request.headers(), "SIP-If-Match").map(|v| v.trim().to_string())
    }

    /// Send 200 OK for PUBLISH with SIP-ETag and Expires.
    async fn send_publish_ok(
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        etag: &str,
        expires: u32,
    ) {
        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("SIP-ETag"),
            smol_str::SmolStr::new(etag),
        );
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new(expires.to_string()),
        );
        handle.send_final(response).await;
    }

    /// Spawn a background task to notify watchers of a state change.
    fn spawn_notify_watchers(&self, aor: &str) {
        let delay = self.presence.notify_delay();
        let handler = SipPresenceHandler {
            presence: self.presence.clone(),
            uac: Arc::clone(&self.uac),
            tenant_id: self.tenant_id.clone(),
            default_expires: self.default_expires,
            subscribe_limiter: Arc::clone(&self.subscribe_limiter),
            publish_limiter: Arc::clone(&self.publish_limiter),
        };
        let aor = aor.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(e) = handler.notify_watchers(&aor).await {
                warn!(error = %e, aor = %aor, "Failed to notify watchers after PUBLISH");
            }
        });
    }

    /// Extract source IP from the top Via header's received param or sent-by host.
    fn extract_source_ip(request: &sip_core::Request) -> String {
        if let Some(via) = sip_parse::header(request.headers(), "Via") {
            // Try to extract "received" parameter first
            if let Some(pos) = via.find("received=") {
                let rest = &via[pos + 9..];
                let end = rest.find(|c: char| c == ';' || c == ',' || c.is_whitespace())
                    .unwrap_or(rest.len());
                return rest[..end].to_string();
            }
            // Fall back to sent-by host (between protocol and ;params)
            // Via: SIP/2.0/UDP 192.168.1.100:5060;branch=...
            if let Some(host_start) = via.rfind(' ') {
                let host_part = &via[host_start + 1..];
                let end = host_part.find(|c: char| c == ';' || c == ',')
                    .unwrap_or(host_part.len());
                let host_port = &host_part[..end];
                // Strip port
                if let Some((host, _)) = host_port.rsplit_once(':') {
                    return host.to_string();
                }
                return host_port.to_string();
            }
        }
        "unknown".to_string()
    }

    /// Process pending NOTIFY retries from the retry queue.
    pub async fn process_retry_queue(&self) {
        let due_entries = self.presence.dequeue_due_retries();
        if due_entries.is_empty() {
            return;
        }

        let (base_delay_ms, max_delay_ms, max_retries) = self.presence.retry_config();

        for entry in due_entries {
            match self
                .send_notify_to_watcher(&entry.watcher, &entry.subscription_state)
                .await
            {
                Ok(()) => {
                    // Success — remove from retry queue
                    self.presence.remove_retry(
                        &entry.watcher.subscription_id,
                        &entry.watcher.presentity_aor,
                    );
                    debug!(
                        watcher = %entry.watcher.watcher_aor,
                        presentity = %entry.watcher.presentity_aor,
                        attempt = entry.attempt,
                        "NOTIFY retry succeeded"
                    );
                }
                Err(_) => {
                    let next_attempt = entry.attempt + 1;
                    if next_attempt > max_retries {
                        warn!(
                            watcher = %entry.watcher.watcher_aor,
                            presentity = %entry.watcher.presentity_aor,
                            attempts = next_attempt,
                            "NOTIFY retry max attempts exceeded, giving up"
                        );
                        // Send giveup NOTIFY before removing
                        let _ = self
                            .send_notify_to_watcher(&entry.watcher, "terminated;reason=giveup")
                            .await;
                        self.presence.remove_retry(
                            &entry.watcher.subscription_id,
                            &entry.watcher.presentity_aor,
                        );
                    } else {
                        // Re-enqueue with exponential backoff
                        let delay_ms = (base_delay_ms * 2u64.pow(next_attempt - 1))
                            .min(max_delay_ms);
                        let retry = fcp_presence::NotifyRetryEntry {
                            watcher: entry.watcher,
                            subscription_state: entry.subscription_state,
                            attempt: next_attempt,
                            next_retry_at: Instant::now() + Duration::from_millis(delay_ms),
                            created_at: entry.created_at,
                        };
                        self.presence.enqueue_retry(retry);
                    }
                }
            }
        }
    }

    /// Force-terminate a specific watcher subscription with `terminated;reason=forced`.
    /// Used by admin APIs to forcibly remove a subscription.
    pub async fn terminate_watcher_forced(
        &self,
        presentity_aor: &str,
        subscription_id: &str,
    ) -> Result<()> {
        if let Some(watcher) = self
            .presence
            .force_terminate_watcher(presentity_aor, subscription_id)
        {
            let _ = self
                .send_notify_to_watcher(&watcher, "terminated;reason=forced")
                .await;
            self.presence
                .remove_watcher(presentity_aor, subscription_id)
                .await;
            info!(
                presentity = %presentity_aor,
                watcher = %watcher.watcher_aor,
                "Watcher force-terminated"
            );
        }
        Ok(())
    }

    /// Remove a presentity and send `terminated;reason=noresource` to all its watchers.
    /// Used by admin APIs when deleting a presentity.
    pub async fn terminate_watchers_noresource(
        &self,
        aor: &str,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let watchers = self
            .presence
            .remove_presence_with_watchers(aor, tenant_id)
            .await;

        let sub_state = "terminated;reason=noresource";
        for watcher in &watchers {
            if let Err(e) = self.send_notify_to_watcher(watcher, sub_state).await {
                warn!(
                    watcher = %watcher.watcher_aor,
                    error = %e,
                    "Failed to send noresource NOTIFY"
                );
            }
        }

        info!(
            aor = %aor,
            watcher_count = watchers.len(),
            "Presentity removed with noresource NOTIFYs"
        );
        Ok(())
    }

    /// Clamp the requested subscription expiry to the configured range.
    fn clamp_subscribe_expires(&self, requested: u32) -> u32 {
        self.presence.cap_subscription_expiry(requested)
    }

    /// Compute Subscription-State "active;expires=N" for a watcher.
    fn subscription_state_active(watcher: &Watcher) -> String {
        let remaining = watcher
            .expires_at
            .saturating_duration_since(Instant::now())
            .as_secs();
        format!("active;expires={remaining}")
    }

    /// Send NOTIFY to a single watcher with the appropriate body and Subscription-State.
    ///
    /// This method generates the XML body per-watcher. For batch notifications
    /// (e.g., notify_watchers), use `send_notify_with_body` to avoid redundant
    /// XML generation.
    async fn send_notify_to_watcher(
        &self,
        watcher: &Watcher,
        subscription_state: &str,
    ) -> Result<()> {
        let (event, content_type, body) = match watcher.event_package.as_str() {
            "dialog" => {
                let xml = self.presence.generate_dialog_info(&watcher.presentity_aor);
                ("dialog", "application/dialog-info+xml", xml)
            }
            "presence.winfo" => {
                let xml = self
                    .presence
                    .generate_watcherinfo(&watcher.presentity_aor)
                    .unwrap_or_else(|e| {
                        warn!(error = %e, "Failed to generate watcherinfo XML");
                        "<watcherinfo xmlns=\"urn:ietf:params:xml:ns:watcherinfo\" version=\"0\" state=\"full\"></watcherinfo>".to_string()
                    });
                ("presence.winfo", "application/watcherinfo+xml", xml)
            }
            _ => {
                let xml = self
                    .presence
                    .generate_pidf(&watcher.presentity_aor)
                    .unwrap_or_else(|| {
                        fcp_presence::pidf::generate_pidf(
                            &watcher.presentity_aor,
                            &fcp_presence::PresenceState::Closed,
                            None,
                            None,
                        )
                    });
                ("presence", "application/pidf+xml", xml)
            }
        };

        self.send_notify_with_body(watcher, subscription_state, event, content_type, &body)
            .await
    }

    /// Send NOTIFY to a single watcher with a pre-built body.
    ///
    /// This is the core send logic used by both single-watcher sends and batch
    /// notifications. It handles the UAC send, retry enqueue, and metrics.
    async fn send_notify_with_body(
        &self,
        watcher: &Watcher,
        subscription_state: &str,
        event: &str,
        content_type: &str,
        body: &str,
    ) -> Result<()> {
        let target_uri = SipUri::parse(&watcher.contact_uri)
            .context("Failed to parse watcher contact URI")?;

        let (base_delay_ms, _, _) = self.presence.retry_config();

        match self
            .uac
            .send_unsolicited_notify_with_state(
                &target_uri,
                event,
                content_type,
                body,
                subscription_state,
            )
            .await
        {
            Ok(response) => {
                if response.code() >= 200 && response.code() < 300 {
                    debug!(
                        watcher = %watcher.watcher_aor,
                        presentity = %watcher.presentity_aor,
                        event = %watcher.event_package,
                        sub_state = %subscription_state,
                        "Presence NOTIFY accepted"
                    );
                    self.presence.metrics().inc_counter("fcp_presence_notify_success_total", &[]);
                    self.presence.remove_retry(
                        &watcher.subscription_id,
                        &watcher.presentity_aor,
                    );
                } else {
                    warn!(
                        watcher = %watcher.watcher_aor,
                        presentity = %watcher.presentity_aor,
                        code = response.code(),
                        "Presence NOTIFY rejected"
                    );
                    self.presence.metrics().inc_counter("fcp_presence_notify_failure_total", &[]);
                    let retry = fcp_presence::NotifyRetryEntry {
                        watcher: watcher.clone(),
                        subscription_state: subscription_state.to_string(),
                        attempt: 1,
                        next_retry_at: Instant::now() + Duration::from_millis(base_delay_ms),
                        created_at: Instant::now(),
                    };
                    self.presence.enqueue_retry(retry);
                }
            }
            Err(e) => {
                warn!(
                    watcher = %watcher.watcher_aor,
                    presentity = %watcher.presentity_aor,
                    error = %e,
                    "Failed to send presence NOTIFY"
                );
                self.presence.metrics().inc_counter("fcp_presence_notify_failure_total", &[]);
                let retry = fcp_presence::NotifyRetryEntry {
                    watcher: watcher.clone(),
                    subscription_state: subscription_state.to_string(),
                    attempt: 1,
                    next_retry_at: Instant::now() + Duration::from_millis(base_delay_ms),
                    created_at: Instant::now(),
                };
                self.presence.enqueue_retry(retry);
            }
        }

        Ok(())
    }

    /// Parse SUBSCRIBE request into a context struct.
    fn parse_subscribe_context(&self, request: &sip_core::Request) -> Result<SubscribeContext> {
        let event = Self::extract_event(request)
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;

        if event != "presence" && event != "dialog" && event != "presence.winfo" {
            bail!("Not a presence/dialog/winfo subscription (event: {event})");
        }

        let presentity_aor = Self::extract_presentity_aor(request)
            .ok_or_else(|| anyhow::anyhow!("Cannot extract presentity AOR from Request-URI"))?;

        let watcher_aor = Self::extract_watcher_aor(request)
            .unwrap_or_else(|| "unknown".to_string());

        let contact_uri = Self::extract_contact_uri(request)
            .ok_or_else(|| anyhow::anyhow!("Missing Contact header in SUBSCRIBE"))?;

        let raw_expires = Self::extract_expires(request).unwrap_or(self.default_expires);
        let expires = self.clamp_subscribe_expires(raw_expires);

        Ok(SubscribeContext {
            event,
            presentity_aor,
            watcher_aor,
            contact_uri,
            expires,
        })
    }

    /// Check authorization for SUBSCRIBE. Sends 403 and returns false if denied.
    async fn check_subscribe_auth(
        &self,
        ctx: &SubscribeContext,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<bool> {
        if !self
            .presence
            .is_watcher_authorized(&ctx.watcher_aor, &ctx.presentity_aor, &ctx.event)
        {
            info!(
                watcher = %ctx.watcher_aor,
                presentity = %ctx.presentity_aor,
                "SUBSCRIBE rejected by auth policy"
            );
            let response =
                sip_uas::UserAgentServer::create_response(request, 403, "Forbidden");
            handle.send_final(response).await;
            return Ok(false);
        }
        Ok(true)
    }

    /// Handle unsubscribe (Expires: 0).
    async fn handle_unsubscribe(
        &self,
        ctx: &SubscribeContext,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        let watchers = self.presence.get_watchers(&ctx.presentity_aor);
        for w in &watchers {
            if w.watcher_aor == ctx.watcher_aor && w.event_package == ctx.event {
                if let Err(e) = self
                    .send_notify_to_watcher(w, "terminated;reason=deactivated")
                    .await
                {
                    warn!(error = %e, "Failed to send terminated NOTIFY on unsubscribe");
                }
                self.presence
                    .remove_watcher(&ctx.presentity_aor, &w.subscription_id)
                    .await;
                break;
            }
        }

        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new("0"),
        );
        handle.send_final(response).await;
        Ok(())
    }

    /// Handle SUBSCRIBE refresh for existing subscription.
    async fn handle_subscribe_refresh(
        &self,
        ctx: &SubscribeContext,
        sub_id: &str,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        let new_expires = Instant::now() + Duration::from_secs(ctx.expires as u64);
        if let Err(e) = self
            .presence
            .refresh_watcher(&ctx.presentity_aor, sub_id, new_expires)
        {
            warn!(error = %e, "Failed to refresh existing watcher");
        }

        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new(ctx.expires.to_string()),
        );
        handle.send_final(response).await;

        // Send NOTIFY with active state for refreshed subscription
        let refreshed_watchers = self.presence.get_watchers(&ctx.presentity_aor);
        if let Some(w) = refreshed_watchers
            .iter()
            .find(|w| w.subscription_id == sub_id)
        {
            let sub_state = Self::subscription_state_active(w);
            let delay = self.presence.notify_delay();
            let handler = SipPresenceHandler {
                presence: self.presence.clone(),
                uac: Arc::clone(&self.uac),
                tenant_id: self.tenant_id.clone(),
                default_expires: self.default_expires,
                subscribe_limiter: Arc::clone(&self.subscribe_limiter),
                publish_limiter: Arc::clone(&self.publish_limiter),
            };
            let watcher_clone = w.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                if let Err(e) = handler
                    .send_notify_to_watcher(&watcher_clone, &sub_state)
                    .await
                {
                    warn!(error = %e, "Failed to send NOTIFY for refreshed subscription");
                }
            });
        }

        Ok(())
    }

    /// Handle watcher-info SUBSCRIBE (RFC 3857/3858).
    async fn handle_winfo_subscribe(
        &self,
        ctx: &SubscribeContext,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        let subscription_id = uuid::Uuid::new_v4().to_string();

        if ctx.expires == 0 {
            // Unsubscribe from winfo
            let winfo_watchers = self.presence.get_winfo_watchers(&ctx.presentity_aor);
            for w in &winfo_watchers {
                if w.watcher_aor == ctx.watcher_aor {
                    self.presence
                        .remove_winfo_watcher(&ctx.presentity_aor, &w.subscription_id);
                    break;
                }
            }
            let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
            let _ = response.headers_mut().push(
                smol_str::SmolStr::new("Expires"),
                smol_str::SmolStr::new("0"),
            );
            handle.send_final(response).await;
            return Ok(());
        }

        let watcher = Watcher {
            watcher_aor: ctx.watcher_aor.clone(),
            presentity_aor: ctx.presentity_aor.clone(),
            subscription_id: subscription_id.clone(),
            expires_at: Instant::now() + Duration::from_secs(ctx.expires as u64),
            tenant_id: self.tenant_id.clone(),
            contact_uri: ctx.contact_uri.clone(),
            event_package: "presence.winfo".to_string(),
            sip_call_id: None,
            sip_from_tag: None,
            sip_to_tag: None,
        };

        self.presence.add_winfo_watcher(watcher.clone());

        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new(ctx.expires.to_string()),
        );
        handle.send_final(response).await;

        // Send initial NOTIFY with watcherinfo XML
        let sub_state = Self::subscription_state_active(&watcher);
        let delay = self.presence.notify_delay();
        let handler = SipPresenceHandler {
            presence: self.presence.clone(),
            uac: Arc::clone(&self.uac),
            tenant_id: self.tenant_id.clone(),
            default_expires: self.default_expires,
            subscribe_limiter: Arc::clone(&self.subscribe_limiter),
            publish_limiter: Arc::clone(&self.publish_limiter),
        };

        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(e) = handler.send_notify_to_watcher(&watcher, &sub_state).await {
                warn!(error = %e, "Failed to send initial winfo NOTIFY");
            }
        });

        Ok(())
    }

    /// Handle new SUBSCRIBE (create watcher, send initial NOTIFY).
    async fn handle_new_subscription(
        &self,
        ctx: &SubscribeContext,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        let subscription_id = uuid::Uuid::new_v4().to_string();

        // Extract SIP dialog context from SUBSCRIBE for in-dialog NOTIFYs
        let sip_call_id = sip_parse::header(request.headers(), "Call-ID")
            .map(|v| v.trim().to_string());
        let sip_from_tag = sip_parse::header(request.headers(), "From")
            .and_then(|from| {
                from.rsplit_once("tag=").map(|(_, tag)| {
                    tag.split(';').next().unwrap_or(tag).trim().to_string()
                })
            });
        // Generate a To-tag for our 200 OK response
        let sip_to_tag = Some(uuid::Uuid::new_v4().simple().to_string());

        let watcher = Watcher {
            watcher_aor: ctx.watcher_aor.clone(),
            presentity_aor: ctx.presentity_aor.clone(),
            subscription_id: subscription_id.clone(),
            expires_at: Instant::now() + Duration::from_secs(ctx.expires as u64),
            tenant_id: self.tenant_id.clone(),
            contact_uri: ctx.contact_uri.clone(),
            event_package: ctx.event.clone(),
            sip_call_id,
            sip_from_tag,
            sip_to_tag,
        };

        if let Err(e) = self.presence.add_watcher(watcher.clone()).await {
            warn!(error = %e, "Failed to add presence watcher");
            let response =
                sip_uas::UserAgentServer::create_response(request, 500, "Server Error");
            handle.send_final(response).await;
            return Ok(());
        }

        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new(ctx.expires.to_string()),
        );
        handle.send_final(response).await;

        // Send initial NOTIFY with current state and active Subscription-State
        let sub_state = Self::subscription_state_active(&watcher);
        let delay = self.presence.notify_delay();
        let handler = SipPresenceHandler {
            presence: self.presence.clone(),
            uac: Arc::clone(&self.uac),
            tenant_id: self.tenant_id.clone(),
            default_expires: self.default_expires,
            subscribe_limiter: Arc::clone(&self.subscribe_limiter),
            publish_limiter: Arc::clone(&self.publish_limiter),
        };

        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(e) = handler.send_notify_to_watcher(&watcher, &sub_state).await {
                warn!(error = %e, "Failed to send initial presence NOTIFY");
            }
        });

        Ok(())
    }
}

#[async_trait::async_trait]
impl fcp_sip::presence::PresenceHandler for SipPresenceHandler {
    async fn handle_subscribe(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        let ctx = self.parse_subscribe_context(request)?;

        // Rate limiting: check both per-IP and per-AOR
        let source_ip = Self::extract_source_ip(request);
        let aor_key = format!("aor:{}", ctx.presentity_aor);
        if !self.subscribe_limiter.check_rate_limit(&source_ip)
            || !self.subscribe_limiter.check_rate_limit(&aor_key)
        {
            warn!(
                source = %source_ip,
                presentity = %ctx.presentity_aor,
                "SUBSCRIBE rate limited"
            );
            self.presence.metrics().inc_counter("fcp_presence_rate_limited_total", &[("type", "subscribe")]);
            let mut response =
                sip_uas::UserAgentServer::create_response(request, 503, "Service Unavailable");
            let _ = response.headers_mut().push(
                smol_str::SmolStr::new("Retry-After"),
                smol_str::SmolStr::new("60"),
            );
            handle.send_final(response).await;
            return Ok(());
        }

        info!(
            presentity = %ctx.presentity_aor,
            watcher = %ctx.watcher_aor,
            event = %ctx.event,
            expires = ctx.expires,
            "Handling presence SUBSCRIBE"
        );

        // Watcher-info event package (RFC 3857/3858)
        if ctx.event == "presence.winfo" {
            if !self.presence.is_winfo_enabled() {
                let response =
                    sip_uas::UserAgentServer::create_response(request, 489, "Bad Event");
                handle.send_final(response).await;
                return Ok(());
            }
            return self.handle_winfo_subscribe(&ctx, request, handle).await;
        }

        if !self.check_subscribe_auth(&ctx, request, handle).await? {
            return Ok(());
        }

        if ctx.expires == 0 {
            return self.handle_unsubscribe(&ctx, request, handle).await;
        }

        if let Some(existing_sub_id) =
            self.presence
                .find_existing_watcher(&ctx.watcher_aor, &ctx.presentity_aor, &ctx.event)
        {
            return self.handle_subscribe_refresh(&ctx, &existing_sub_id, request, handle).await;
        }

        self.handle_new_subscription(&ctx, request, handle).await
    }

    async fn handle_publish(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()> {
        // Check Event header — only handle "presence"
        let event = Self::extract_event(request)
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;

        if event != "presence" {
            bail!("Not a presence PUBLISH (event: {event})");
        }

        // Rate limiting: check both per-IP and per-AOR
        let source_ip = Self::extract_source_ip(request);
        let presentity_aor = Self::extract_presentity_aor(request)
            .unwrap_or_default();
        let aor_key = format!("aor:{presentity_aor}");
        if !self.publish_limiter.check_rate_limit(&source_ip)
            || !self.publish_limiter.check_rate_limit(&aor_key)
        {
            warn!(
                source = %source_ip,
                presentity = %presentity_aor,
                "PUBLISH rate limited"
            );
            self.presence.metrics().inc_counter("fcp_presence_rate_limited_total", &[("type", "publish")]);
            let mut response =
                sip_uas::UserAgentServer::create_response(request, 503, "Service Unavailable");
            let _ = response.headers_mut().push(
                smol_str::SmolStr::new("Retry-After"),
                smol_str::SmolStr::new("60"),
            );
            handle.send_final(response).await;
            return Ok(());
        }

        let if_match = Self::extract_sip_if_match(request);
        let expires = Self::extract_expires(request).unwrap_or(3600);
        let expires = self.presence.cap_publish_expiry(expires);
        let body = std::str::from_utf8(request.body())
            .context("PUBLISH body is not valid UTF-8")?;
        let has_body = !body.is_empty();

        match (has_body, &if_match) {
            // Initial PUBLISH: body present, no SIP-If-Match
            (true, None) => {
                let update = self
                    .presence
                    .parse_pidf(body)
                    .context("Failed to parse PIDF XML from PUBLISH body")?;

                info!(
                    aor = %update.aor,
                    state = %update.state,
                    "Handling initial PUBLISH"
                );

                let etag = self.presence.store_publish(
                    &update.aor,
                    update.state,
                    update.note.clone(),
                    &self.tenant_id,
                    expires,
                ).await;

                // Update aggregated presence state (if composition is disabled,
                // this is the primary update; if enabled, store_publish already recomposed)
                let info = self
                    .presence
                    .update_presence(update, &self.tenant_id)
                    .await
                    .context("Failed to update presence from PUBLISH")?;

                fcp_presence::events::publish_received(
                    &None,
                    &info.aor,
                    info.state,
                    &self.tenant_id,
                )
                .await;

                Self::send_publish_ok(request, handle, &etag, expires).await;
                self.spawn_notify_watchers(&info.aor);
            }

            // Refresh PUBLISH: no body, SIP-If-Match present
            (false, Some(old_etag)) => {
                if expires == 0 {
                    // Remove PUBLISH
                    match self.presence.remove_publish(old_etag).await {
                        Ok(()) => {
                            debug!(etag = %old_etag, "PUBLISH removed (Expires: 0)");
                            let mut response =
                                sip_uas::UserAgentServer::create_response(request, 200, "OK");
                            let _ = response.headers_mut().push(
                                smol_str::SmolStr::new("Expires"),
                                smol_str::SmolStr::new("0"),
                            );
                            handle.send_final(response).await;
                        }
                        Err(_) => {
                            let response = sip_uas::UserAgentServer::create_response(
                                request,
                                412,
                                "Conditional Request Failed",
                            );
                            handle.send_final(response).await;
                        }
                    }
                } else {
                    // Refresh expiry
                    match self.presence.refresh_publish(old_etag, expires) {
                        Ok(new_etag) => {
                            Self::send_publish_ok(request, handle, &new_etag, expires).await;
                        }
                        Err(_) => {
                            let response = sip_uas::UserAgentServer::create_response(
                                request,
                                412,
                                "Conditional Request Failed",
                            );
                            handle.send_final(response).await;
                        }
                    }
                }
            }

            // Modify PUBLISH: body present, SIP-If-Match present
            (true, Some(old_etag)) => {
                let update = self
                    .presence
                    .parse_pidf(body)
                    .context("Failed to parse PIDF XML from PUBLISH body")?;

                info!(
                    aor = %update.aor,
                    state = %update.state,
                    etag = %old_etag,
                    "Handling modify PUBLISH"
                );

                match self.presence.modify_publish(
                    old_etag,
                    update.state,
                    update.note.clone(),
                    expires,
                ).await {
                    Ok(new_etag) => {
                        let info = self
                            .presence
                            .update_presence(update, &self.tenant_id)
                            .await
                            .context("Failed to update presence from PUBLISH")?;

                        fcp_presence::events::publish_received(
                            &None,
                            &info.aor,
                            info.state,
                            &self.tenant_id,
                        )
                        .await;

                        Self::send_publish_ok(request, handle, &new_etag, expires).await;
                        self.spawn_notify_watchers(&info.aor);
                    }
                    Err(_) => {
                        let response = sip_uas::UserAgentServer::create_response(
                            request,
                            412,
                            "Conditional Request Failed",
                        );
                        handle.send_final(response).await;
                    }
                }
            }

            // No body, no SIP-If-Match → meaningless per RFC 3903
            (false, None) => {
                let response = sip_uas::UserAgentServer::create_response(
                    request,
                    400,
                    "Bad Request",
                );
                handle.send_final(response).await;
            }
        }

        Ok(())
    }

    async fn notify_watchers(&self, aor: &str) -> Result<()> {
        let watchers = self.presence.get_watchers(aor);

        if watchers.is_empty() {
            return Ok(());
        }

        debug!(
            aor = %aor,
            count = watchers.len(),
            "Notifying watchers of presence change"
        );

        // Pre-generate XML bodies once per event package to avoid redundant
        // regeneration across N watchers for the same AOR.
        // This also ensures dialog-info version increments once per state change
        // (correct BLF semantics) instead of N times for N watchers.
        let pidf_body = self.presence.generate_pidf(aor).unwrap_or_else(|| {
            fcp_presence::pidf::generate_pidf(
                aor,
                &fcp_presence::PresenceState::Closed,
                None,
                None,
            )
        });
        let dialog_body = self.presence.generate_dialog_info(aor);
        let winfo_body = self
            .presence
            .generate_watcherinfo(aor)
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to generate watcherinfo XML");
                "<watcherinfo xmlns=\"urn:ietf:params:xml:ns:watcherinfo\" version=\"0\" state=\"full\"></watcherinfo>".to_string()
            });

        for watcher in &watchers {
            let sub_state = Self::subscription_state_active(watcher);
            let (event, content_type, body) = match watcher.event_package.as_str() {
                "dialog" => ("dialog", "application/dialog-info+xml", &dialog_body),
                "presence.winfo" => (
                    "presence.winfo",
                    "application/watcherinfo+xml",
                    &winfo_body,
                ),
                _ => ("presence", "application/pidf+xml", &pidf_body),
            };
            if let Err(e) = self
                .send_notify_with_body(watcher, &sub_state, event, content_type, body)
                .await
            {
                warn!(
                    watcher = %watcher.watcher_aor,
                    error = %e,
                    "Failed to notify watcher"
                );
            }
        }

        Ok(())
    }

    async fn send_terminated_notifies(
        &self,
        watchers: &[fcp_presence::Watcher],
        reason: &str,
    ) -> Result<()> {
        let sub_state = format!("terminated;reason={reason}");
        for watcher in watchers {
            if let Err(e) = self.send_notify_to_watcher(watcher, &sub_state).await {
                warn!(
                    watcher = %watcher.watcher_aor,
                    error = %e,
                    "Failed to send terminated NOTIFY"
                );
            }
        }
        Ok(())
    }
}
