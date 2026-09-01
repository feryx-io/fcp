//! Call bridge manager — tracks WebRTC-to-SIP call pairs and drives call lifecycle.
//!
//! Each `CallBridge` represents a single browser→SIP phone call:
//! - The browser side is a WebRTC PeerConnection (managed by forge-api)
//! - The SIP side is an outbound INVITE (driven by the IntegratedUAC)
//! - Media is bridged through a forge-media session (Opus ↔ G.711 transcoding)

use dashmap::DashMap;
use fcp_core::{CallId, DialogId};
use fcp_media::{BackendType, MediaConfig, MediaManager};
use fcp_sip::uac::SipUserAgentClient;
use forge_api::routes::webrtc::WebRtcManager;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// State of a bridged call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallBridgeState {
    Initiating,
    Ringing,
    Connected,
    Terminating,
    Terminated,
    Failed(String),
}

impl std::fmt::Display for CallBridgeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initiating => write!(f, "initiating"),
            Self::Ringing => write!(f, "ringing"),
            Self::Connected => write!(f, "connected"),
            Self::Terminating => write!(f, "terminating"),
            Self::Terminated => write!(f, "terminated"),
            Self::Failed(reason) => write!(f, "failed: {}", reason),
        }
    }
}

/// A single WebRTC↔SIP bridged call.
pub struct CallBridge {
    pub id: String,
    pub webrtc_connection_id: String,
    pub media_call_id: Option<CallId>,
    #[allow(dead_code)]
    pub dialog_id: Option<DialogId>,
    pub state: CallBridgeState,
    pub destination_uri: String,
    pub created_at: Instant,
    /// SIP dialog for BYE (set after 200 OK)
    pub sip_dialog: Option<sip_dialog::Dialog>,
}

/// Serializable call info returned by the HTTP API.
#[derive(Debug, Serialize, Deserialize)]
pub struct CallInfo {
    pub id: String,
    pub webrtc_connection_id: String,
    pub destination_uri: String,
    pub state: CallBridgeState,
    pub duration_secs: f64,
}

impl CallInfo {
    pub fn from_bridge(bridge: &CallBridge) -> Self {
        Self {
            id: bridge.id.clone(),
            webrtc_connection_id: bridge.webrtc_connection_id.clone(),
            destination_uri: bridge.destination_uri.clone(),
            state: bridge.state.clone(),
            duration_secs: bridge.created_at.elapsed().as_secs_f64(),
        }
    }
}

/// Manages all active WebRTC↔SIP call bridges.
pub struct CallBridgeManager {
    calls: DashMap<String, Arc<RwLock<CallBridge>>>,
    uac: Arc<SipUserAgentClient>,
    webrtc_manager: Arc<WebRtcManager>,
    media_manager: Arc<MediaManager>,
    public_ip: String,
}

impl CallBridgeManager {
    pub fn new(
        uac: Arc<SipUserAgentClient>,
        webrtc_manager: Arc<WebRtcManager>,
        media_manager: Arc<MediaManager>,
        public_ip: String,
    ) -> Self {
        Self {
            calls: DashMap::new(),
            uac,
            webrtc_manager,
            media_manager,
            public_ip,
        }
    }

    /// Initiate a call from a WebRTC connection to a SIP destination.
    pub async fn initiate_call(
        &self,
        connection_id: String,
        destination: String,
    ) -> anyhow::Result<CallInfo> {
        // 1. Verify WebRTC connection exists
        if self.webrtc_manager.get_connection(&connection_id).is_none() {
            anyhow::bail!("WebRTC connection '{}' not found", connection_id);
        }

        // 2. Create identifiers
        let call_id = CallId::new();
        let dialog_id = DialogId::new();
        let bridge_id = format!("call-{}", uuid::Uuid::new_v4());

        info!(
            bridge_id = %bridge_id,
            connection_id = %connection_id,
            destination = %destination,
            "Initiating WebRTC→SIP call"
        );

        // 3. Create media session
        let media_session = self
            .media_manager
            .create_session_default(call_id, dialog_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create media session: {}", e))?;

        let (rtp_port, _rtcp_port) = media_session.ports();

        // 4. Build SDP offer for the SIP side (PCMU/PCMA, RTP/AVP)
        let sdp_offer = format!(
            "v=0\r\n\
             o=fcp-webrtc-gw 0 0 IN IP4 {ip}\r\n\
             s=WebRTC Gateway Call\r\n\
             c=IN IP4 {ip}\r\n\
             t=0 0\r\n\
             m=audio {port} RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=sendrecv\r\n",
            ip = self.public_ip,
            port = rtp_port,
        );

        // 5. Create the bridge record
        let bridge = CallBridge {
            id: bridge_id.clone(),
            webrtc_connection_id: connection_id,
            media_call_id: Some(call_id),
            dialog_id: Some(dialog_id),
            state: CallBridgeState::Initiating,
            destination_uri: destination.clone(),
            created_at: Instant::now(),
            sip_dialog: None,
        };
        let bridge_arc = Arc::new(RwLock::new(bridge));
        self.calls.insert(bridge_id.clone(), bridge_arc.clone());

        // 6. Send SIP INVITE
        let call_handle = self
            .uac
            .invite(&destination, Some(&sdp_offer))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send SIP INVITE: {}", e))?;

        // 7. Spawn async task to drive the call lifecycle
        let uac = self.uac.clone();
        let media_mgr = self.media_manager.clone();
        let bridge_for_task = bridge_arc.clone();
        let calls_map = self.calls.clone();
        let bid = bridge_id.clone();

        tokio::spawn(async move {
            drive_call_lifecycle(call_handle, bridge_for_task, uac, media_mgr, calls_map, bid)
                .await;
        });

        // 8. Return initial call info
        let info = {
            let b = bridge_arc.read().await;
            CallInfo::from_bridge(&b)
        };
        Ok(info)
    }

    /// Hang up an active call.
    pub async fn hangup(&self, call_id: &str) -> anyhow::Result<()> {
        let bridge_arc = self
            .calls
            .get(call_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("Call '{}' not found", call_id))?;

        let mut bridge = bridge_arc.write().await;

        match bridge.state {
            CallBridgeState::Connected => {
                bridge.state = CallBridgeState::Terminating;
                info!(call_id = %call_id, "Sending BYE");

                if let Some(ref dialog) = bridge.sip_dialog {
                    if let Err(e) = self.uac.bye(dialog).await {
                        warn!(call_id = %call_id, error = %e, "Failed to send BYE");
                    }
                }

                // Terminate media
                if let Some(ref media_cid) = bridge.media_call_id {
                    let _ = self.media_manager.terminate_session(media_cid).await;
                }

                bridge.state = CallBridgeState::Terminated;
            }
            CallBridgeState::Initiating | CallBridgeState::Ringing => {
                // Call not yet connected — just mark as terminated
                bridge.state = CallBridgeState::Terminated;
                if let Some(ref media_cid) = bridge.media_call_id {
                    let _ = self.media_manager.terminate_session(media_cid).await;
                }
            }
            _ => {
                anyhow::bail!(
                    "Call '{}' is in state {} and cannot be hung up",
                    call_id,
                    bridge.state
                );
            }
        }

        Ok(())
    }

    /// List all calls.
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        let mut result = Vec::with_capacity(self.calls.len());
        for entry in self.calls.iter() {
            let bridge = entry.value().read().await;
            result.push(CallInfo::from_bridge(&bridge));
        }
        result
    }

    /// Get a single call by ID.
    pub async fn get_call(&self, call_id: &str) -> Option<CallInfo> {
        let bridge_arc = self.calls.get(call_id)?.value().clone();
        let bridge = bridge_arc.read().await;
        Some(CallInfo::from_bridge(&bridge))
    }
}

/// Create a media manager from gateway configuration.
pub async fn create_media_manager(
    config: &crate::config::GatewayConfig,
) -> anyhow::Result<Arc<MediaManager>> {
    let media_config = MediaConfig {
        backend: BackendType::Embedded,
        rtp_port_start: config.media.rtp_port_start,
        rtp_port_end: config.media.rtp_port_end,
        public_ip: config.media.public_ip.clone(),
        ..Default::default()
    };

    let manager = MediaManager::new(media_config)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create media manager: {}", e))?;

    info!(
        rtp_ports = format!(
            "{}-{}",
            config.media.rtp_port_start, config.media.rtp_port_end
        ),
        "Media manager created"
    );

    Ok(Arc::new(manager))
}

/// Async task that drives SIP call lifecycle (provisionals → final → media setup).
async fn drive_call_lifecycle(
    call_handle: sip_uac::integrated::CallHandle,
    bridge: Arc<RwLock<CallBridge>>,
    _uac: Arc<SipUserAgentClient>,
    media_mgr: Arc<MediaManager>,
    _calls_map: DashMap<String, Arc<RwLock<CallBridge>>>,
    bridge_id: String,
) {
    // 1. Wait for provisionals (180 Ringing, 183 Session Progress)
    loop {
        tokio::select! {
            provisional = call_handle.await_provisional() => {
                match provisional {
                    Some(resp) => {
                        let code = resp.start_line().code();
                        info!(bridge_id = %bridge_id, code = code, "SIP provisional response");
                        if code == 180 || code == 183 {
                            let mut b = bridge.write().await;
                            if b.state == CallBridgeState::Initiating {
                                b.state = CallBridgeState::Ringing;
                            }
                        }
                    }
                    None => {
                        // No more provisionals — final response available
                        break;
                    }
                }
            }
        }
    }

    // 2. Await final response
    let final_response = match call_handle.await_final().await {
        Ok(resp) => resp,
        Err(e) => {
            error!(bridge_id = %bridge_id, error = %e, "SIP INVITE failed");
            let mut b = bridge.write().await;
            b.state = CallBridgeState::Failed(e.to_string());
            cleanup_media(&media_mgr, &b).await;
            return;
        }
    };

    let code = final_response.start_line().code();
    info!(bridge_id = %bridge_id, code = code, "SIP final response");

    if !(200..300).contains(&code) {
        // Non-2xx final → call failed
        let reason = format!("{} {}", code, final_response.start_line().reason());
        error!(bridge_id = %bridge_id, reason = %reason, "SIP call rejected");
        let mut b = bridge.write().await;
        b.state = CallBridgeState::Failed(reason);
        cleanup_media(&media_mgr, &b).await;
        return;
    }

    // 3. 200 OK — extract SDP answer and set up media B-leg
    let sdp_answer = std::str::from_utf8(final_response.body())
        .unwrap_or("")
        .to_string();

    // Store dialog for BYE
    {
        let dialog = call_handle.dialog.read().await.clone();
        let mut b = bridge.write().await;
        b.sip_dialog = Some(dialog);
    }

    // Update media B-leg with remote SDP
    {
        let b = bridge.read().await;
        if let Some(ref media_cid) = b.media_call_id {
            if !sdp_answer.is_empty() {
                if let Err(e) = media_mgr.update_session_b_leg(media_cid, sdp_answer).await {
                    warn!(bridge_id = %bridge_id, error = %e, "Failed to update media B-leg");
                }
            }
        }
    }

    // 4. Mark as connected
    {
        let mut b = bridge.write().await;
        b.state = CallBridgeState::Connected;
    }
    info!(bridge_id = %bridge_id, "Call connected (WebRTC ↔ SIP)");
}

/// Clean up media session on failure.
async fn cleanup_media(media_mgr: &MediaManager, bridge: &CallBridge) {
    if let Some(ref media_cid) = bridge.media_call_id {
        let _ = media_mgr.terminate_session(media_cid).await;
    }
}
