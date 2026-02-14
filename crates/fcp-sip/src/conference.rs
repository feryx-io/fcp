//! Conference call handler trait for routing SIP INVITEs to conference rooms.
//!
//! This trait is implemented by `fcp-conference-server`'s `ConferenceCallHandler`
//! and plugged into `SipMessageHandler` to intercept INVITEs destined for
//! conference room numbers before the normal B2BUA / flow routing takes over.

use anyhow::Result;
use std::net::SocketAddr;

/// Trait for handling conference-bound SIP calls.
///
/// Implementors check whether an incoming INVITE targets a conference room
/// and, if so, handle the full lifecycle (answer, PIN collection, mixer join).
#[async_trait::async_trait]
pub trait ConferenceHandler: Send + Sync {
    /// Returns `true` if this request should be handled as a conference call
    /// (i.e., the Request-URI user part matches a known meeting room number).
    async fn can_handle(&self, request: &sip_core::Request) -> bool;

    /// Handle an INVITE for a conference room.
    ///
    /// The implementor is responsible for:
    /// - Looking up the meeting configuration
    /// - Sending provisional/final SIP responses via `handle`
    /// - Setting up RTP and the audio bridge
    /// - PIN authentication (if required)
    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        peer: SocketAddr,
    ) -> Result<()>;

    /// Clean up when a BYE is received for a conference participant.
    ///
    /// Called with the SIP Call-ID so the handler can remove the participant
    /// from the mixer and abort the audio bridge task.
    ///
    /// This is intentionally **not** async because `SipMessageHandler::handle_bye`
    /// is synchronous. Implementations should perform only cheap, non-blocking
    /// cleanup (e.g. remove from DashMap, abort a JoinHandle).
    ///
    /// Returns `Ok(())` if this call_id belonged to a conference session and was
    /// cleaned up. Returns `Err` if the call_id is unknown (not a conference call),
    /// so the caller can fall through to normal B2BUA BYE handling.
    fn handle_bye(&self, call_id: &str) -> Result<()>;
}
