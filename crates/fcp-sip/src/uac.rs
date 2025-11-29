//! User Agent Client (UAC) for initiating outbound SIP calls.
//!
//! This module provides a thin wrapper around siphon-rs IntegratedUAC.

use anyhow::Result;
use sip_dialog::Dialog;
use sip_uac::integrated::{CallHandle, IntegratedUAC};
use std::sync::Arc;

/// User Agent Client for initiating outbound calls.
///
/// This is a thin wrapper around siphon-rs IntegratedUAC that provides:
/// - INVITE initiation with automatic transaction management
/// - Automatic ACK generation for 200 OK
/// - BYE generation for call termination
/// - Automatic DNS resolution (RFC 3263)
/// - Automatic authentication retry (401/407)
pub struct SipUserAgentClient {
    /// Siphon-rs integrated UAC with full transaction management
    inner: Arc<IntegratedUAC>,
}

impl SipUserAgentClient {
    /// Create a new UAC from an IntegratedUAC.
    ///
    /// # Arguments
    /// * `integrated_uac` - The siphon-rs IntegratedUAC instance
    pub fn new(integrated_uac: Arc<IntegratedUAC>) -> Self {
        Self {
            inner: integrated_uac,
        }
    }

    /// Get a reference to the underlying IntegratedUAC.
    ///
    /// This allows access to all IntegratedUAC methods directly.
    pub fn inner(&self) -> &Arc<IntegratedUAC> {
        &self.inner
    }

    /// Initiate an outbound INVITE to establish a call.
    ///
    /// # Arguments
    /// * `target` - URI of the callee (e.g., "sip:bob@example.com")
    /// * `sdp_body` - Optional SDP offer (None for late offer)
    ///
    /// # Returns
    /// A `CallHandle` that can be used to:
    /// - Receive provisional responses (100, 180, 183)
    /// - Receive the final response (200 OK, 486 Busy, etc.)
    /// - Send CANCEL if needed
    /// - Access the created dialog
    ///
    /// # Example
    /// ```no_run
    /// # use fcp_sip::prelude::*;
    /// # async fn example(uac: &SipUserAgentClient) -> anyhow::Result<()> {
    /// // Early offer (INVITE with SDP)
    /// let sdp = "v=0\r\no=- 123 0 IN IP4 192.168.1.1\r\n...";
    /// let call = uac.invite("sip:bob@example.com", Some(sdp)).await?;
    ///
    /// // Wait for provisional responses
    /// // (access call.provisional_rx)
    ///
    /// // Wait for final response
    /// // (access call.final_rx)
    ///
    /// // If successful, dialog is in call.dialog
    /// # Ok(())
    /// # }
    /// ```
    pub async fn invite(&self, target: &str, sdp_body: Option<&str>) -> Result<CallHandle> {
        self.inner.invite(target, sdp_body).await
    }

    /// Send BYE to terminate an established call.
    ///
    /// # Arguments
    /// * `dialog` - The dialog to terminate (obtained from CallHandle)
    ///
    /// # Returns
    /// The final response to the BYE request (typically 200 OK)
    ///
    /// # Example
    /// ```no_run
    /// # use fcp_sip::prelude::*;
    /// # use sip_dialog::Dialog;
    /// # async fn example(uac: &SipUserAgentClient, dialog: &Dialog) -> anyhow::Result<()> {
    /// let response = uac.bye(dialog).await?;
    /// println!("Call terminated: {} {}", response.start.code, response.start.reason);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bye(&self, dialog: &Dialog) -> Result<sip_core::Response> {
        self.inner.bye(dialog).await
    }

    /// Send REGISTER request.
    ///
    /// # Arguments
    /// * `registrar_uri` - URI of the registrar (e.g., "sip:example.com")
    /// * `expires` - Registration duration in seconds (0 to unregister)
    ///
    /// # Returns
    /// The final response (200 OK on success, 401 if auth fails)
    ///
    /// Note: IntegratedUAC handles automatic authentication retry
    pub async fn register(
        &self,
        registrar_uri: &str,
        expires: u32,
    ) -> Result<sip_core::Response> {
        self.inner.register(registrar_uri, Some(expires)).await
    }
}

/// Re-export key types from siphon-rs for convenience
pub use sip_uac::integrated::CallHandle as OutboundCall;
