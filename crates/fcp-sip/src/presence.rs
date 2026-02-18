//! Presence handler trait for SIP SUBSCRIBE/PUBLISH/NOTIFY.
//!
//! This trait is implemented by `fcp-call-manager`'s `SipPresenceHandler`
//! and plugged into `SipMessageHandler` to intercept:
//! - Out-of-dialog SUBSCRIBE for `event: presence` and `event: dialog` (BLF)
//! - PUBLISH for `event: presence` (RFC 3903)
//!
//! The handler manages watcher subscriptions and sends NOTIFY when presence
//! state changes.

use anyhow::Result;

/// Trait for handling SIP presence subscriptions, publications, and notifications.
///
/// Implementors handle the SIP signaling side of presence:
/// - Accept SUBSCRIBE requests, track watchers, send initial NOTIFY
/// - Accept PUBLISH requests, update presence state
/// - Send NOTIFY to all watchers when state changes
#[async_trait::async_trait]
pub trait PresenceHandler: Send + Sync {
    /// Handle an out-of-dialog SUBSCRIBE for `event: presence` or `event: dialog`.
    ///
    /// The implementor is responsible for:
    /// - Checking the Event header matches "presence" or "dialog"
    /// - Extracting the presentity AOR from Request-URI
    /// - Sending 200 OK with Expires header via `handle`
    /// - Adding a watcher subscription
    /// - Sending an initial NOTIFY with current state (PIDF or dialog-info)
    ///
    /// Returns `Ok(())` if handled, `Err` if not a presence/dialog subscription
    /// (allowing fallback to B2BUA in-dialog handling).
    async fn handle_subscribe(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()>;

    /// Handle a PUBLISH request for `event: presence` (RFC 3903).
    ///
    /// The implementor is responsible for:
    /// - Parsing the PIDF XML body
    /// - Updating the presentity's state
    /// - Sending 200 OK with SIP-ETag via `handle`
    /// - Triggering NOTIFY to all watchers
    ///
    /// Returns `Ok(())` if handled, `Err` if not a presence PUBLISH.
    async fn handle_publish(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
    ) -> Result<()>;

    /// Notify all watchers of a presentity about a state change.
    ///
    /// Called when presence state changes due to:
    /// - Call start/end (auto phone state)
    /// - Registration/unregistration
    /// - HTTP API update
    /// - PUBLISH from user agent
    ///
    /// Sends NOTIFY with appropriate body (PIDF for `event: presence`,
    /// dialog-info for `event: dialog`) to each watcher's Contact URI.
    async fn notify_watchers(&self, aor: &str) -> Result<()>;

    /// Send terminated NOTIFY to a list of watchers (e.g., on expiry or shutdown).
    ///
    /// The `reason` is placed in the Subscription-State header as
    /// `terminated;reason={reason}` per RFC 3265 §3.2.4.
    async fn send_terminated_notifies(
        &self,
        _watchers: &[fcp_presence::Watcher],
        _reason: &str,
    ) -> Result<()> {
        Ok(())
    }
}
