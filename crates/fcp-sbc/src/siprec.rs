//! SIPREC recording session management for the SBC (RFC 7866).
//!
//! Creates and manages SIPREC recording sessions by sending INVITEs with
//! multipart/mixed bodies (SDP + RFC 7865 metadata XML) to configured
//! recording servers.

use dashmap::DashMap;
use fcp_core::CallId;
use fcp_sbc_core::siprec::{RecordingPolicy, RecordingServer, SiprecConfig};
use fcp_sip::uac::SipUserAgentClient;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::warn;
use uuid::Uuid;

/// Pending call info captured at INVITE, consumed when call is answered.
struct PendingCallInfo {
    caller_aor: String,
    callee_aor: String,
}

/// Active recording session.
struct RecordingSession {
    recording_id: String,
    _call_id: CallId,
    sip_call_id: String,
    server_uri: String,
    _started_at: SystemTime,
}

/// SIPREC recording session manager.
///
/// Hooks into call lifecycle events to start/stop recording sessions
/// on configured SIPREC recording servers.
pub struct SiprecRecorder {
    config: SiprecConfig,
    uac: Arc<SipUserAgentClient>,
    pending_calls: DashMap<String, PendingCallInfo>,
    sessions: DashMap<String, RecordingSession>,
}

impl SiprecRecorder {
    /// Create a new recorder. Returns `None` if policy is `None` or no
    /// enabled servers are configured.
    pub fn new(config: SiprecConfig, uac: Arc<SipUserAgentClient>) -> Option<Self> {
        if config.policy == RecordingPolicy::None {
            return None;
        }
        if !config.servers.iter().any(|s| s.enabled) {
            return None;
        }
        Some(Self {
            config,
            uac,
            pending_calls: DashMap::new(),
            sessions: DashMap::new(),
        })
    }

    /// Select the best recording server (lowest priority among enabled).
    fn select_server(&self) -> Option<&RecordingServer> {
        self.config
            .servers
            .iter()
            .filter(|s| s.enabled)
            .min_by_key(|s| s.priority)
    }

    /// Called when a new call is initiated. Stores caller/callee info for
    /// later use when the call is answered.
    pub fn on_call_initiated(&self, call_id: &CallId, from: &str, to: &str) {
        self.pending_calls.insert(
            call_id.to_string(),
            PendingCallInfo {
                caller_aor: from.to_string(),
                callee_aor: to.to_string(),
            },
        );
    }

    /// Start a recording session for an answered call.
    ///
    /// Consumes pending call info, builds metadata XML and multipart body,
    /// and sends a SIPREC INVITE to the recording server. Failures are logged
    /// but never propagate to the call.
    pub async fn start_recording(&self, call_id: &CallId) {
        let call_key = call_id.to_string();
        let pending = match self.pending_calls.remove(&call_key) {
            Some((_, info)) => info,
            None => {
                warn!(call_id = %call_key, "No pending call info for recording start");
                return;
            }
        };

        let server = match self.select_server() {
            Some(s) => s,
            None => {
                warn!("No enabled recording server available");
                return;
            }
        };

        let recording_id = Uuid::new_v4().to_string();
        let session_id = Uuid::new_v4().to_string();
        let boundary = format!("srec-{}", &recording_id[..8]);

        // Build SDP for recording (placeholder — the recording server will
        // provide the actual RTP targets in its answer)
        let sdp = build_recording_sdp(&session_id);

        // Build RFC 7865 metadata XML
        let metadata = generate_metadata_xml(
            &recording_id,
            &pending.caller_aor,
            &pending.callee_aor,
        );

        // Build multipart/mixed body
        let body = build_multipart_body(&boundary, &sdp, &metadata);
        let content_type = format!("multipart/mixed;boundary={}", boundary);

        match self
            .uac
            .send_siprec_invite(&server.uri, &body, &content_type)
            .await
        {
            Ok(call_handle) => {
                // Extract SIP Call-ID from the INVITE request
                let sip_call_id = call_handle
                    .invite_request()
                    .headers()
                    .get_smol("Call-ID")
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                self.sessions.insert(
                    call_key,
                    RecordingSession {
                        recording_id,
                        _call_id: call_id.clone(),
                        sip_call_id,
                        server_uri: server.uri.clone(),
                        _started_at: SystemTime::now(),
                    },
                );
            }
            Err(e) => {
                warn!(
                    call_id = %call_key,
                    error = %e,
                    server = %server.uri,
                    "Failed to start SIPREC recording session"
                );
            }
        }
    }

    /// Stop a recording session by sending BYE to the recording server.
    pub async fn stop_recording(&self, call_id: &CallId) {
        let call_key = call_id.to_string();
        if let Some((_, session)) = self.sessions.remove(&call_key) {
            // Attempt to tear down the SIPREC dialog by looking up the dialog
            // by the SIP Call-ID. If we can't find it or BYE fails, just log.
            let dialog_mgr = self.uac.inner().dialog_manager();
            if let Some(dialog_mgr) = dialog_mgr {
                if let Some(dialog) = dialog_mgr.find_by_call_id(&session.sip_call_id) {
                    if let Err(e) = self.uac.bye(&dialog).await {
                        warn!(
                            call_id = %call_key,
                            sip_call_id = %session.sip_call_id,
                            server = %session.server_uri,
                            error = %e,
                            "Failed to send BYE for SIPREC session"
                        );
                    }
                }
            }
        }
    }

    /// Remove pending call info on call failure (call never answered).
    pub fn cleanup_pending(&self, call_id: &CallId) {
        self.pending_calls.remove(&call_id.to_string());
    }
}

/// Build a minimal SDP offer for the recording session.
fn build_recording_sdp(session_id: &str) -> String {
    format!(
        "v=0\r\n\
         o=- {} 1 IN IP4 0.0.0.0\r\n\
         s=SIPREC\r\n\
         c=IN IP4 0.0.0.0\r\n\
         t=0 0\r\n\
         m=audio 0 RTP/AVP 0\r\n\
         a=recvonly\r\n",
        session_id
    )
}

/// Generate RFC 7865 recording metadata XML.
pub(crate) fn generate_metadata_xml(
    recording_id: &str,
    caller_aor: &str,
    callee_aor: &str,
) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Caller and callee participant IDs
    let caller_id = Uuid::new_v4();
    let callee_id = Uuid::new_v4();
    let stream_id = Uuid::new_v4();

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
         <recording xmlns=\"urn:ietf:params:xml:ns:recording:1\">\r\n\
         <datamode>complete</datamode>\r\n\
         <session id=\"{recording_id}\">\r\n\
           <start-time>{now}</start-time>\r\n\
         </session>\r\n\
         <participant id=\"{caller_id}\">\r\n\
           <nameID aor=\"{caller_aor}\"/>\r\n\
           <send>{stream_id}</send>\r\n\
         </participant>\r\n\
         <participant id=\"{callee_id}\">\r\n\
           <nameID aor=\"{callee_aor}\"/>\r\n\
           <recv>{stream_id}</recv>\r\n\
         </participant>\r\n\
         <stream id=\"{stream_id}\">\r\n\
           <label>audio</label>\r\n\
         </stream>\r\n\
         </recording>\r\n"
    )
}

/// Build a multipart/mixed body with SDP and metadata XML parts.
pub(crate) fn build_multipart_body(boundary: &str, sdp: &str, metadata: &str) -> String {
    format!(
        "--{boundary}\r\n\
         Content-Type: application/sdp\r\n\
         \r\n\
         {sdp}\r\n\
         --{boundary}\r\n\
         Content-Type: application/rs-metadata+xml\r\n\
         \r\n\
         {metadata}\r\n\
         --{boundary}--"
    )
}

/// Check whether a SIPREC config would produce a recorder (without
/// needing a UAC). Used for testing policy/server selection logic.
pub(crate) fn would_create_recorder(config: &SiprecConfig) -> bool {
    config.policy != RecordingPolicy::None && config.servers.iter().any(|s| s.enabled)
}

/// Select recording server from config (standalone version for testing).
pub(crate) fn select_server_from_config(config: &SiprecConfig) -> Option<&RecordingServer> {
    config
        .servers
        .iter()
        .filter(|s| s.enabled)
        .min_by_key(|s| s.priority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fcp_sbc_core::siprec::{RecordingPolicy, RecordingServer, SiprecConfig};

    fn make_config(policy: RecordingPolicy) -> SiprecConfig {
        SiprecConfig {
            policy,
            servers: vec![
                RecordingServer {
                    name: "recorder-primary".into(),
                    uri: "sip:recorder@primary.example.com".into(),
                    enabled: true,
                    priority: 10,
                },
                RecordingServer {
                    name: "recorder-backup".into(),
                    uri: "sip:recorder@backup.example.com".into(),
                    enabled: true,
                    priority: 20,
                },
            ],
            include_metadata: true,
        }
    }

    #[test]
    fn test_recorder_not_created_for_none_policy() {
        let config = SiprecConfig::default(); // policy = None
        assert!(!would_create_recorder(&config));
    }

    #[test]
    fn test_recorder_created_for_all_policy() {
        let config = make_config(RecordingPolicy::All);
        assert!(would_create_recorder(&config));
    }

    #[test]
    fn test_select_server_lowest_priority() {
        let config = make_config(RecordingPolicy::All);
        let server = select_server_from_config(&config).unwrap();
        assert_eq!(server.priority, 10);
        assert_eq!(server.name, "recorder-primary");
    }

    #[test]
    fn test_select_server_skips_disabled() {
        let config = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: vec![
                RecordingServer {
                    name: "disabled".into(),
                    uri: "sip:recorder@disabled.example.com".into(),
                    enabled: false,
                    priority: 1,
                },
                RecordingServer {
                    name: "enabled".into(),
                    uri: "sip:recorder@enabled.example.com".into(),
                    enabled: true,
                    priority: 10,
                },
            ],
            include_metadata: true,
        };
        let server = select_server_from_config(&config).unwrap();
        assert_eq!(server.name, "enabled");
    }

    #[test]
    fn test_select_server_none_available() {
        let config = SiprecConfig {
            policy: RecordingPolicy::All,
            servers: vec![RecordingServer {
                name: "disabled".into(),
                uri: "sip:recorder@example.com".into(),
                enabled: false,
                priority: 1,
            }],
            include_metadata: true,
        };
        assert!(!would_create_recorder(&config));
    }

    #[test]
    fn test_generate_metadata_valid_xml() {
        let xml = generate_metadata_xml("rec-123", "sip:alice@example.com", "sip:bob@example.com");
        assert!(xml.contains("<recording"));
        assert!(xml.contains("</recording>"));
        assert!(xml.contains("rec-123"));
        assert!(xml.contains("sip:alice@example.com"));
        assert!(xml.contains("sip:bob@example.com"));
        assert!(xml.contains("<session"));
        assert!(xml.contains("<participant"));
        assert!(xml.contains("<stream"));
        assert!(xml.contains("<start-time>"));
        assert!(xml.contains("<send>"));
        assert!(xml.contains("<recv>"));
    }

    #[test]
    fn test_build_multipart_body_structure() {
        let sdp = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\n";
        let metadata = "<recording/>\r\n";
        let body = build_multipart_body("myboundary", sdp, metadata);
        // Must contain both part boundaries and closing boundary
        assert!(body.contains("--myboundary\r\n"));
        assert!(body.contains("--myboundary--"));
        // Must contain correct Content-Types
        assert!(body.contains("Content-Type: application/sdp"));
        assert!(body.contains("Content-Type: application/rs-metadata+xml"));
        // Must contain both parts
        assert!(body.contains(sdp));
        assert!(body.contains(metadata));
    }

    #[test]
    fn test_build_recording_sdp() {
        let sdp = build_recording_sdp("sess-42");
        assert!(sdp.contains("v=0"));
        assert!(sdp.contains("sess-42"));
        assert!(sdp.contains("SIPREC"));
        assert!(sdp.contains("a=recvonly"));
    }
}
