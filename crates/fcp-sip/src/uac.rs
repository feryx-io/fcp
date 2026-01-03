//! User Agent Client (UAC) for initiating outbound SIP calls.
//!
//! This module provides a thin wrapper around siphon-rs IntegratedUAC.

use crate::settings::ServerSettings;
use anyhow::Result;
use sip_dialog::{Dialog, Subscription};
use sip_uac::integrated::{CallHandle, IntegratedUAC};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
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
    transcript_dir: Option<PathBuf>,
    _metrics: crate::metrics::MetricsHandle,
}

impl SipUserAgentClient {
    /// Create a new UAC from an IntegratedUAC.
    ///
    /// # Arguments
    /// * `integrated_uac` - The siphon-rs IntegratedUAC instance
    pub fn new(integrated_uac: Arc<IntegratedUAC>) -> Self {
        Self {
            inner: integrated_uac,
            transcript_dir: None,
            _metrics: crate::metrics::noop_metrics(),
        }
    }

    /// Create a new UAC with a transcript directory for SIP logging.
    pub fn with_transcript_dir(
        integrated_uac: Arc<IntegratedUAC>,
        transcript_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            inner: integrated_uac,
            transcript_dir,
            _metrics: crate::metrics::noop_metrics(),
        }
    }

    pub fn from_settings(integrated_uac: Arc<IntegratedUAC>, settings: &ServerSettings) -> Self {
        Self {
            inner: integrated_uac,
            transcript_dir: settings.transcript_dir.clone(),
            _metrics: settings.metrics.clone(),
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
    /// println!("Call terminated: {} {}", response.start_line().code(), response.start_line().reason());
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
    pub async fn register(&self, registrar_uri: &str, expires: u32) -> Result<sip_core::Response> {
        self.inner.register(registrar_uri, Some(expires)).await
    }

    /// Send an in-dialog UPDATE.
    pub async fn send_update(
        &self,
        dialog: &mut Dialog,
        sdp_body: Option<&str>,
    ) -> Result<sip_core::Response> {
        self.log_outbound("UPDATE", Some(dialog.id()));
        self.inner.send_update(dialog, sdp_body).await
    }

    /// Send an in-dialog re-INVITE.
    pub async fn send_reinvite(
        &self,
        dialog: &mut Dialog,
        sdp_body: Option<&str>,
    ) -> Result<sip_core::Response> {
        self.log_outbound("INVITE", Some(dialog.id()));
        self.inner.send_reinvite(dialog, sdp_body).await
    }

    /// Send an in-dialog INFO.
    pub async fn send_info(
        &self,
        dialog: &mut Dialog,
        content_type: &str,
        body: &str,
    ) -> Result<sip_core::Response> {
        self.log_outbound("INFO", Some(dialog.id()));
        self.inner.send_info(dialog, content_type, body).await
    }

    /// Send an in-dialog REFER.
    pub async fn send_refer(
        &self,
        dialog: &mut Dialog,
        refer_to: &sip_core::SipUri,
        target_dialog: Option<&Dialog>,
    ) -> Result<(sip_core::Response, Option<Subscription>)> {
        self.log_outbound("REFER", Some(dialog.id()));
        self.inner.send_refer(dialog, refer_to, target_dialog).await
    }

    /// Send PRACK for a reliable provisional.
    pub async fn send_prack(
        &self,
        dialog: &mut Dialog,
        provisional: &sip_core::Response,
    ) -> Result<sip_core::Response> {
        self.log_outbound("PRACK", Some(dialog.id()));
        self.inner.send_prack(dialog, provisional).await
    }

    /// Send SUBSCRIBE within a dialog.
    pub async fn send_subscribe(
        &self,
        dialog: &mut Dialog,
        event: &str,
        expires: Option<u32>,
    ) -> Result<(sip_core::Response, Option<Subscription>)> {
        self.log_outbound("SUBSCRIBE", Some(&dialog.id()));
        // Target is remote target of dialog
        self.inner
            .subscribe(dialog.remote_target().clone(), event, expires)
            .await
    }

    /// Send NOTIFY for a subscription.
    pub async fn send_notify(
        &self,
        subscription: &Subscription,
        state: sip_dialog::SubscriptionState,
        body: Option<&str>,
    ) -> Result<sip_core::Response> {
        let dialog_id = sip_dialog::DialogId::new(
            subscription.id().call_id().clone(),
            subscription.id().from_tag().clone(),
            subscription.id().to_tag().clone(),
        );
        self.log_outbound("NOTIFY", Some(&dialog_id.unwrap()));
        self.inner.notify(subscription, state, body).await
    }

    /// Refresh an active dialog per RFC 4028.
    pub async fn refresh_session(
        &self,
        dialog: &mut Dialog,
        session_expires: u32,
        refresher: &str,
        use_update: bool,
        sdp_body: Option<&str>,
    ) -> Result<sip_core::Response> {
        self.inner
            .refresh_session(dialog, session_expires, refresher, use_update, sdp_body)
            .await
    }

    fn log_outbound(&self, method: &str, dialog_id: Option<&sip_dialog::DialogId>) {
        if let Some(dir) = &self.transcript_dir {
            if let Some(did) = dialog_id {
                let call_id = did.call_id();
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join(format!("{}.log", call_id.replace(':', "_"))))
                {
                    let _ = writeln!(
                        file,
                        "{:?} outbound {} call_id={} dialog={:?}",
                        std::time::SystemTime::now(),
                        method,
                        call_id,
                        did
                    );
                }
            }
        }
    }
}

/// Re-export key types from siphon-rs for convenience
pub use sip_uac::integrated::CallHandle as OutboundCall;
