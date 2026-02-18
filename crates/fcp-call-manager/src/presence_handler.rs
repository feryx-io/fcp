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
}

impl SipPresenceHandler {
    pub fn new(
        presence: PresenceManager,
        uac: Arc<SipUserAgentClient>,
        tenant_id: TenantId,
        default_expires: u32,
    ) -> Self {
        Self {
            presence,
            uac,
            tenant_id,
            default_expires,
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

    /// Send NOTIFY to a single watcher with the appropriate body.
    async fn send_notify_to_watcher(
        &self,
        watcher: &Watcher,
    ) -> Result<()> {
        let target_uri = SipUri::parse(&watcher.contact_uri)
            .context("Failed to parse watcher contact URI")?;

        let (event, content_type, body) = match watcher.event_package.as_str() {
            "dialog" => {
                let xml = self.presence.generate_dialog_info(&watcher.presentity_aor);
                ("dialog", "application/dialog-info+xml", xml)
            }
            _ => {
                // Default to presence PIDF
                let xml = self
                    .presence
                    .generate_pidf(&watcher.presentity_aor)
                    .unwrap_or_else(|| {
                        // Generate a "closed" PIDF if no presentity found
                        fcp_presence::pidf::generate_pidf(
                            &watcher.presentity_aor,
                            &fcp_presence::PresenceState::Closed,
                            None,
                        )
                    });
                ("presence", "application/pidf+xml", xml)
            }
        };

        match self
            .uac
            .send_unsolicited_notify(&target_uri, event, content_type, &body)
            .await
        {
            Ok(response) => {
                if response.code() >= 200 && response.code() < 300 {
                    debug!(
                        watcher = %watcher.watcher_aor,
                        presentity = %watcher.presentity_aor,
                        event = %watcher.event_package,
                        "Presence NOTIFY accepted"
                    );
                } else {
                    warn!(
                        watcher = %watcher.watcher_aor,
                        presentity = %watcher.presentity_aor,
                        code = response.code(),
                        "Presence NOTIFY rejected"
                    );
                }
            }
            Err(e) => {
                warn!(
                    watcher = %watcher.watcher_aor,
                    presentity = %watcher.presentity_aor,
                    error = %e,
                    "Failed to send presence NOTIFY"
                );
            }
        }

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
        // Check Event header — only handle "presence" and "dialog"
        let event = Self::extract_event(request)
            .ok_or_else(|| anyhow::anyhow!("Missing Event header"))?;

        if event != "presence" && event != "dialog" {
            bail!("Not a presence/dialog subscription (event: {event})");
        }

        let presentity_aor = Self::extract_presentity_aor(request)
            .ok_or_else(|| anyhow::anyhow!("Cannot extract presentity AOR from Request-URI"))?;

        let watcher_aor = Self::extract_watcher_aor(request)
            .unwrap_or_else(|| "unknown".to_string());

        let contact_uri = Self::extract_contact_uri(request)
            .ok_or_else(|| anyhow::anyhow!("Missing Contact header in SUBSCRIBE"))?;

        let expires = Self::extract_expires(request).unwrap_or(self.default_expires);

        info!(
            presentity = %presentity_aor,
            watcher = %watcher_aor,
            event = %event,
            expires = expires,
            "Handling presence SUBSCRIBE"
        );

        // Check for unsubscribe (Expires: 0)
        if expires == 0 {
            // Remove watcher — find by watcher_aor + presentity_aor
            let watchers = self.presence.get_watchers(&presentity_aor);
            for w in watchers {
                if w.watcher_aor == watcher_aor && w.event_package == event {
                    self.presence.remove_watcher(&presentity_aor, &w.subscription_id).await;
                    break;
                }
            }

            // Send 200 OK with Expires: 0
            let response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
            handle.send_final(response).await;
            return Ok(());
        }

        // Generate a subscription ID
        let subscription_id = uuid::Uuid::new_v4().to_string();

        // Add watcher
        let watcher = Watcher {
            watcher_aor: watcher_aor.clone(),
            presentity_aor: presentity_aor.clone(),
            subscription_id: subscription_id.clone(),
            expires_at: Instant::now() + Duration::from_secs(expires as u64),
            tenant_id: self.tenant_id.clone(),
            contact_uri: contact_uri.clone(),
            event_package: event.clone(),
        };

        if let Err(e) = self.presence.add_watcher(watcher.clone()).await {
            warn!(error = %e, "Failed to add presence watcher");
            let response = sip_uas::UserAgentServer::create_response(request, 500, "Server Error");
            handle.send_final(response).await;
            return Ok(());
        }

        // Send 200 OK with Expires header
        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new(expires.to_string()),
        );
        handle.send_final(response).await;

        // Send initial NOTIFY with current state
        // Small delay to let the 200 OK arrive first
        let handler = SipPresenceHandler {
            presence: self.presence.clone(),
            uac: Arc::clone(&self.uac),
            tenant_id: self.tenant_id.clone(),
            default_expires: self.default_expires,
        };

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Err(e) = handler.send_notify_to_watcher(&watcher).await {
                warn!(error = %e, "Failed to send initial presence NOTIFY");
            }
        });

        Ok(())
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

        let body = std::str::from_utf8(request.body())
            .context("PUBLISH body is not valid UTF-8")?;

        if body.is_empty() {
            // RFC 3903: PUBLISH with empty body is a refresh (keep alive)
            // Respond 200 OK with a new SIP-ETag
            let etag = uuid::Uuid::new_v4().simple().to_string();
            let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
            let _ = response.headers_mut().push(
                smol_str::SmolStr::new("SIP-ETag"),
                smol_str::SmolStr::new(&etag),
            );
            let _ = response.headers_mut().push(
                smol_str::SmolStr::new("Expires"),
                smol_str::SmolStr::new(
                    Self::extract_expires(request)
                        .unwrap_or(3600)
                        .to_string(),
                ),
            );
            handle.send_final(response).await;
            return Ok(());
        }

        // Parse PIDF XML from body
        let update = self
            .presence
            .parse_pidf(body)
            .context("Failed to parse PIDF XML from PUBLISH body")?;

        info!(
            aor = %update.aor,
            state = %update.state,
            "Handling presence PUBLISH"
        );

        // Update presence state
        let info = self
            .presence
            .update_presence(update.clone(), &self.tenant_id)
            .await
            .context("Failed to update presence from PUBLISH")?;

        // Publish PUBLISH-received event
        fcp_presence::events::publish_received(
            &None, // Event is already published by update_presence via StatusChanged
            &info.aor,
            info.state,
            &self.tenant_id,
        )
        .await;

        // Send 200 OK with SIP-ETag
        let etag = uuid::Uuid::new_v4().simple().to_string();
        let mut response = sip_uas::UserAgentServer::create_response(request, 200, "OK");
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("SIP-ETag"),
            smol_str::SmolStr::new(&etag),
        );
        let _ = response.headers_mut().push(
            smol_str::SmolStr::new("Expires"),
            smol_str::SmolStr::new(
                Self::extract_expires(request)
                    .unwrap_or(3600)
                    .to_string(),
            ),
        );
        handle.send_final(response).await;

        // Notify all watchers about the state change
        let aor = info.aor.clone();
        let handler = SipPresenceHandler {
            presence: self.presence.clone(),
            uac: Arc::clone(&self.uac),
            tenant_id: self.tenant_id.clone(),
            default_expires: self.default_expires,
        };

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Err(e) = handler.notify_watchers(&aor).await {
                warn!(error = %e, aor = %aor, "Failed to notify watchers after PUBLISH");
            }
        });

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

        for watcher in &watchers {
            if let Err(e) = self.send_notify_to_watcher(watcher).await {
                warn!(
                    watcher = %watcher.watcher_aor,
                    error = %e,
                    "Failed to notify watcher"
                );
            }
        }

        Ok(())
    }
}
