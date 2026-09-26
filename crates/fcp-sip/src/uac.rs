//! User Agent Client (UAC) for initiating outbound SIP calls.
//!
//! This module provides a thin wrapper around siphon-rs IntegratedUAC.

use crate::settings::ServerSettings;
use anyhow::Result;
use sip_core::Headers;
use sip_dialog::{Dialog, Subscription};
use sip_uac::integrated::{CallHandle, Flow, IntegratedUAC};
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

    /// INVITE with extra headers when given, plain INVITE otherwise.
    pub async fn invite_with_optional_headers(
        &self,
        target: &str,
        sdp_body: Option<&str>,
        extra_headers: Option<Headers>,
    ) -> Result<CallHandle> {
        match extra_headers {
            Some(headers) => self.invite_with_headers(target, sdp_body, headers).await,
            None => self.invite(target, sdp_body).await,
        }
    }

    /// Initiate an outbound INVITE with additional SIP headers.
    pub async fn invite_with_headers(
        &self,
        target: &str,
        sdp_body: Option<&str>,
        extra_headers: Headers,
    ) -> Result<CallHandle> {
        self.inner
            .invite_with_headers(target, sdp_body, extra_headers)
            .await
    }

    /// An INVITE carrying what `options` names — any body with its content
    /// type, extra headers and a per-call From together (an emergency call's
    /// SDP and PIDF-LO, `Geolocation`, the callback number as caller).
    pub async fn invite_with_options(
        &self,
        target: &str,
        options: sip_uac::InviteOptions,
    ) -> Result<CallHandle> {
        self.inner.invite_with_options(target, options).await
    }

    /// [`Self::invite_with_options`] through an existing connection
    /// ([`Self::invite_via_flow`]).
    pub async fn invite_via_flow_with_options(
        &self,
        target: &str,
        options: sip_uac::InviteOptions,
        flow_stream: tokio::sync::mpsc::Sender<bytes::Bytes>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<CallHandle> {
        self.inner
            .invite_via_flow_with_options(target, options, Flow::new(flow_stream, peer_addr))
            .await
    }

    /// Send an INVITE using an existing connection (RFC 5626 flow support).
    ///
    /// This method allows sending INVITEs to TLS/TCP clients that registered
    /// through an inbound connection (NAT traversal scenario). Instead of
    /// opening a new connection to the client's ephemeral port, the INVITE
    /// is sent through the existing connection identified by the flow stream.
    ///
    /// # Arguments
    /// * `target` - URI of the callee (e.g., "sip:bob@example.com")
    /// * `sdp_body` - Optional SDP offer (None for late offer)
    /// * `flow_stream` - The mpsc::Sender for the existing connection
    /// * `peer_addr` - The peer's socket address for the TransportContext
    pub async fn invite_via_flow(
        &self,
        target: &str,
        sdp_body: Option<&str>,
        flow_stream: tokio::sync::mpsc::Sender<bytes::Bytes>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<CallHandle> {
        self.inner
            .invite_via_flow(target, sdp_body, Flow::new(flow_stream, peer_addr))
            .await
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

    /// Send BYE using an existing connection (RFC 5626 flow support).
    ///
    /// This method allows sending BYE to TLS/TCP clients that registered
    /// through an inbound connection (NAT traversal scenario). Instead of
    /// opening a new connection to the client's ephemeral port, the BYE
    /// is sent through the existing connection identified by the flow stream.
    ///
    /// # Arguments
    /// * `dialog` - The dialog to terminate
    /// * `flow_stream` - The mpsc::Sender for the existing connection
    /// * `peer_addr` - The peer's socket address for the TransportContext
    pub async fn bye_via_flow(
        &self,
        dialog: &Dialog,
        flow_stream: tokio::sync::mpsc::Sender<bytes::Bytes>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<sip_core::Response> {
        self.inner
            .bye_via_flow(dialog, Flow::new(flow_stream, peer_addr))
            .await
    }

    /// Send a BYE that says why the platform, rather than a party, ended
    /// the call: an RFC 3326 `Reason`
    /// (`docs/CAPACITY_AND_SURVIVABILITY.md` §6).
    pub async fn bye_with_reason(
        &self,
        dialog: &Dialog,
        reason: &str,
    ) -> Result<sip_core::Response> {
        self.inner
            .bye_with_headers(dialog, &[("Reason", reason)])
            .await
    }

    /// [`bye_with_reason`](Self::bye_with_reason) over an existing
    /// connection (RFC 5626 flow support).
    pub async fn bye_via_flow_with_reason(
        &self,
        dialog: &Dialog,
        flow_stream: tokio::sync::mpsc::Sender<bytes::Bytes>,
        peer_addr: std::net::SocketAddr,
        reason: &str,
    ) -> Result<sip_core::Response> {
        self.inner
            .bye_via_flow_with_headers(
                dialog,
                Flow::new(flow_stream, peer_addr),
                &[("Reason", reason)],
            )
            .await
    }

    /// A re-INVITE down an existing connection — the flow a site's link
    /// registered over (`docs/HYBRID_CONTROL_PLANE.md` §3.6), which is
    /// the only way to reach it.
    pub async fn send_reinvite_via_flow(
        &self,
        dialog: &mut Dialog,
        sdp_body: Option<&str>,
        flow_stream: tokio::sync::mpsc::Sender<bytes::Bytes>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<sip_core::Response> {
        self.inner
            .send_reinvite_via_flow(dialog, sdp_body, Flow::new(flow_stream, peer_addr))
            .await
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

    /// REGISTER an account other than this node's own — a carrier trunk's
    /// (`docs/TENANT_TRUNKS.md` §8) — answering a challenge with that
    /// account's own credentials. `Contact` still names this node: the
    /// binding points here.
    pub async fn register_as(
        &self,
        account: &str,
        registrar_uri: &str,
        expires: u32,
        credentials: Option<(String, String)>,
    ) -> Result<sip_core::Response> {
        let account = sip_core::SipUri::parse(account)
            .map_err(|e| anyhow::anyhow!("trunk account {account}: {e}"))?;
        self.inner
            .register_as(&account, registrar_uri, Some(expires), credentials)
            .await
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
        self.log_outbound("SUBSCRIBE", Some(dialog.id()));
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
            subscription.id().call_id(),
            subscription.id().from_tag(),
            subscription.id().to_tag(),
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

    /// Send a SIPREC INVITE with custom body and Content-Type.
    ///
    /// Used for initiating SIPREC recording sessions with multipart/mixed bodies
    /// containing SDP and metadata XML.
    pub async fn send_siprec_invite(
        &self,
        target: &str,
        body: &str,
        content_type: &str,
    ) -> Result<CallHandle> {
        self.inner
            .invite_with_body(target, body, content_type)
            .await
    }

    /// Send an unsolicited NOTIFY (e.g., for MWI per RFC 3842).
    pub async fn send_unsolicited_notify(
        &self,
        target: &sip_core::SipUri,
        event: &str,
        content_type: &str,
        body: &str,
    ) -> Result<sip_core::Response> {
        self.inner
            .send_unsolicited_notify(target, event, content_type, body)
            .await
    }

    /// Send an unsolicited NOTIFY with a custom Subscription-State header.
    pub async fn send_unsolicited_notify_with_state(
        &self,
        target: &sip_core::SipUri,
        event: &str,
        content_type: &str,
        body: &str,
        subscription_state: &str,
    ) -> Result<sip_core::Response> {
        self.inner
            .send_unsolicited_notify_with_state(
                target,
                event,
                content_type,
                body,
                subscription_state,
            )
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
