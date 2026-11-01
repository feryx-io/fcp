//! Queue/ACD handler trait for routing SIP INVITEs to ACD queues.
//!
//! This trait is implemented by `fcp-call-manager`'s `QueueCallHandler`
//! and plugged into `SipMessageHandler` to intercept INVITEs destined for
//! queue extensions before the normal flow / B2BUA routing takes over.

use anyhow::Result;
use std::net::SocketAddr;

/// Trait for handling queue-bound SIP calls.
///
/// Implementors check whether an incoming INVITE targets a queue extension
/// and, if so, handle the full lifecycle (answer, MOH, enqueue, distribute).
#[async_trait::async_trait]
pub trait QueueHandler: Send + Sync {
    /// Returns `true` if this request should be handled as a queue call
    /// (i.e., the Request-URI user part matches a queue extension of the
    /// call's tenant, which the B2BUA has already decided).
    async fn can_handle(&self, request: &sip_core::Request, tenant_id: &fcp_core::TenantId)
        -> bool;

    /// Handle an INVITE for a queue extension.
    ///
    /// The implementor is responsible for:
    /// - Looking up the queue configuration
    /// - Sending provisional/final SIP responses via `handle`
    /// - Setting up RTP and MOH playback
    /// - Enqueuing the call and starting the distribution loop
    /// `call_id` is the B2BUA call the INVITE created. The implementor ends
    /// it (`CallManager::terminate_call_with`) when it refuses the caller
    /// and when the caller's queue session ends, so the call's record is
    /// written and its queue record carries the same id
    /// (`docs/CDR_PIPELINE.md` §5).
    async fn handle_invite(
        &self,
        request: &sip_core::Request,
        handle: &sip_transaction::ServerTransactionHandle,
        peer: SocketAddr,
        call_id: fcp_b2bua_core::CallId,
    ) -> Result<()>;

    /// Clean up when a BYE is received for a queued caller or agent.
    ///
    /// Called with the SIP Call-ID so the handler can remove the caller from
    /// the queue, stop MOH, and deallocate RTP resources.
    ///
    /// Returns `Ok(())` if this call_id belonged to a queue session and was
    /// cleaned up. Returns `Err` if the call_id is unknown (not a queue call),
    /// so the caller can fall through to normal B2BUA BYE handling.
    fn handle_bye(&self, call_id: &str) -> Result<()>;

    /// Clean up when a CANCEL is received for a caller entering a queue.
    ///
    /// Called with the SIP Call-ID. Removes any in-progress queue session
    /// (MOH, RTP, queue position).
    /// Returns `Ok(true)` if this call_id was a queue session and was cancelled.
    /// Returns `Ok(false)` if unknown (not a queue call).
    fn handle_cancel(&self, call_id: &str) -> Result<bool>;

    /// Log an agent into their configured queue(s) via feature code.
    ///
    /// The extension is looked up in queue configs to find which queues
    /// include this extension in their agents list.
    async fn agent_login(&self, extension: &str, tenant_id: &fcp_core::TenantId) -> Result<()>;

    /// Log an agent out of all of its tenant's queues via feature code.
    async fn agent_logout(&self, extension: &str, tenant_id: &fcp_core::TenantId) -> Result<()>;

    /// The extension a tenant's queue answers at, the queue named by its
    /// name or its extension; `None` when this node has no such queue. A
    /// flow's Queue node dials the caller there.
    fn extension_of(&self, tenant_id: &fcp_core::TenantId, queue: &str) -> Option<String> {
        let _ = (tenant_id, queue);
        None
    }
}
