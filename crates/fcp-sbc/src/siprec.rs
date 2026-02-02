//! SIPREC recording session management for the SBC (RFC 7866).
//!
//! Creates and manages SIPREC recording sessions by sending INVITEs with
//! multipart/mixed bodies (SDP + RFC 7865 metadata XML) to configured
//! recording servers.

use dashmap::DashMap;
use fcp_core::CallId;
use fcp_events::CallDirection;
use fcp_sbc_core::siprec::{RecordingPolicy, RecordingServer, SiprecConfig};
use fcp_sip::uac::SipUserAgentClient;
use regex::Regex;
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

/// A pre-compiled selective recording rule ready for matching.
struct CompiledRecordingRule {
    #[allow(dead_code)]
    name: String,
    caller_regex: Option<Regex>,
    callee_regex: Option<Regex>,
    trunk_names: Option<Vec<String>>,
    direction: Option<CallDirection>,
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
    compiled_rules: Vec<CompiledRecordingRule>,
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

        // Pre-compile selective recording rules
        let compiled_rules: Vec<CompiledRecordingRule> = config
            .rules
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| {
                let caller_regex = r.caller_pattern.as_ref().and_then(|p| {
                    Regex::new(p)
                        .map_err(|e| warn!(rule = %r.name, pattern = %p, error = %e, "invalid caller_pattern regex"))
                        .ok()
                });
                let callee_regex = r.callee_pattern.as_ref().and_then(|p| {
                    Regex::new(p)
                        .map_err(|e| warn!(rule = %r.name, pattern = %p, error = %e, "invalid callee_pattern regex"))
                        .ok()
                });
                let direction = r.direction.as_deref().and_then(|d| match d {
                    "inbound" => Some(CallDirection::Inbound),
                    "outbound" => Some(CallDirection::Outbound),
                    _ => {
                        warn!(rule = %r.name, direction = %d, "invalid direction in recording rule");
                        None
                    }
                });
                Some(CompiledRecordingRule {
                    name: r.name.clone(),
                    caller_regex,
                    callee_regex,
                    trunk_names: r.trunk_names.clone(),
                    direction,
                })
            })
            .collect();

        Some(Self {
            config,
            uac,
            pending_calls: DashMap::new(),
            sessions: DashMap::new(),
            compiled_rules,
        })
    }

    /// Check whether a call should be recorded based on the configured policy.
    ///
    /// Returns `true` for `All` policy, and checks compiled rules for `Selective`.
    pub fn should_record(
        &self,
        from: &str,
        to: &str,
        direction: &CallDirection,
        trunk_name: Option<&str>,
    ) -> bool {
        match self.config.policy {
            RecordingPolicy::None => false,
            RecordingPolicy::All => true,
            RecordingPolicy::Selective => self
                .compiled_rules
                .iter()
                .any(|rule| rule_matches(rule, from, to, direction, trunk_name)),
        }
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
    ///
    /// For `Selective` policy the call is only tracked (and later recorded)
    /// when it matches at least one enabled rule.
    pub fn on_call_initiated(
        &self,
        call_id: &CallId,
        from: &str,
        to: &str,
        direction: &CallDirection,
        trunk_name: Option<&str>,
    ) {
        if !self.should_record(from, to, direction, trunk_name) {
            return;
        }
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
    ///
    /// `a_leg_sdp` and `b_leg_sdp` are the negotiated SDP bodies from the
    /// call legs, used to extract the actual codec for the recording SDP.
    pub async fn start_recording(
        &self,
        call_id: &CallId,
        a_leg_sdp: Option<&str>,
        b_leg_sdp: Option<&str>,
    ) {
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

        // Build SDP for recording — uses negotiated codec from call legs
        // when available, falls back to PCMU. The recording server will
        // provide the actual RTP targets in its answer.
        let sdp = build_recording_sdp(&session_id, a_leg_sdp, b_leg_sdp);

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

/// Check whether a compiled recording rule matches the given call parameters.
///
/// All non-`None` fields must match (AND logic). Fields that are `None` are
/// treated as wildcards (always match).
fn rule_matches(
    rule: &CompiledRecordingRule,
    from: &str,
    to: &str,
    direction: &CallDirection,
    trunk_name: Option<&str>,
) -> bool {
    // Direction filter
    if let Some(ref rule_dir) = rule.direction {
        if rule_dir != direction {
            return false;
        }
    }
    // Trunk name filter
    if let Some(ref names) = rule.trunk_names {
        match trunk_name {
            Some(tn) => {
                if !names.iter().any(|n| n == tn) {
                    return false;
                }
            }
            None => return false, // rule requires a trunk but call has none
        }
    }
    // Caller pattern
    if let Some(ref re) = rule.caller_regex {
        if !re.is_match(from) {
            return false;
        }
    }
    // Callee pattern
    if let Some(ref re) = rule.callee_regex {
        if !re.is_match(to) {
            return false;
        }
    }
    true
}

/// Extract the first codec from an SDP body.
///
/// Parses `a=rtpmap:<pt> <name>/<rate>` lines and returns
/// `(codec_name, payload_type, clock_rate)` for the first match.
fn extract_first_codec(sdp_text: &str) -> Option<(String, u8, u32)> {
    for line in sdp_text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            // Format: "PT codec/rate" or "PT codec/rate/channels"
            let mut parts = rest.splitn(2, ' ');
            let pt_str = parts.next()?;
            let codec_part = parts.next()?;
            let pt: u8 = pt_str.parse().ok()?;
            let mut codec_parts = codec_part.split('/');
            let name = codec_parts.next()?;
            let rate: u32 = codec_parts.next().and_then(|r| r.parse().ok()).unwrap_or(8000);
            return Some((name.to_string(), pt, rate));
        }
    }
    None
}

/// Build a SIPREC SDP offer for the recording session.
///
/// Per RFC 7866, the SRC (us) sends media to the SRS, so each stream
/// uses `a=sendonly`. Two `m=audio` lines are included — one per
/// participant (a-leg, b-leg) — with `a=label:` attributes.
///
/// When negotiated SDP is available, the actual codec is extracted;
/// otherwise PCMU/8000 (payload type 0) is used as a safe fallback.
fn build_recording_sdp(
    session_id: &str,
    a_leg_sdp: Option<&str>,
    b_leg_sdp: Option<&str>,
) -> String {
    // Try to extract codec from either leg's negotiated SDP
    let (codec_name, pt, rate) = a_leg_sdp
        .and_then(extract_first_codec)
        .or_else(|| b_leg_sdp.and_then(extract_first_codec))
        .unwrap_or_else(|| ("PCMU".to_string(), 0, 8000));

    format!(
        "v=0\r\n\
         o=- {session_id} 1 IN IP4 0.0.0.0\r\n\
         s=SIPREC\r\n\
         c=IN IP4 0.0.0.0\r\n\
         t=0 0\r\n\
         m=audio 0 RTP/AVP {pt}\r\n\
         a=rtpmap:{pt} {codec_name}/{rate}\r\n\
         a=sendonly\r\n\
         a=label:a-leg\r\n\
         m=audio 0 RTP/AVP {pt}\r\n\
         a=rtpmap:{pt} {codec_name}/{rate}\r\n\
         a=sendonly\r\n\
         a=label:b-leg\r\n"
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
            rules: vec![],
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
            rules: vec![],
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
            rules: vec![],
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
        let sdp = build_recording_sdp("sess-42", None, None);
        assert!(sdp.contains("v=0"));
        assert!(sdp.contains("sess-42"));
        assert!(sdp.contains("SIPREC"));
        // RFC 7866: SRC sends to SRS, so sendonly
        assert!(sdp.contains("a=sendonly"));
        // Fallback to PCMU
        assert!(sdp.contains("PCMU/8000"));
        // Two m= lines (a-leg + b-leg)
        assert!(sdp.contains("a=label:a-leg"));
        assert!(sdp.contains("a=label:b-leg"));
    }

    #[test]
    fn test_build_recording_sdp_with_negotiated() {
        let a_sdp = "v=0\r\n\
                      o=- 1 1 IN IP4 192.168.1.1\r\n\
                      s=call\r\n\
                      c=IN IP4 192.168.1.1\r\n\
                      t=0 0\r\n\
                      m=audio 5004 RTP/AVP 9\r\n\
                      a=rtpmap:9 G722/8000\r\n";
        let sdp = build_recording_sdp("sess-99", Some(a_sdp), None);
        assert!(sdp.contains("G722/8000"));
        assert!(sdp.contains("a=sendonly"));
        assert!(sdp.contains("a=label:a-leg"));
        assert!(sdp.contains("a=label:b-leg"));
        // Should use pt 9
        assert!(sdp.contains("RTP/AVP 9"));
    }

    #[test]
    fn test_build_recording_sdp_fallback_to_pcmu() {
        // SDP with no rtpmap lines
        let bad_sdp = "v=0\r\nm=audio 5004 RTP/AVP 0\r\n";
        let sdp = build_recording_sdp("sess-00", Some(bad_sdp), None);
        assert!(sdp.contains("PCMU/8000"));
        assert!(sdp.contains("RTP/AVP 0"));
    }

    #[test]
    fn test_extract_first_codec() {
        let sdp = "v=0\r\n\
                    m=audio 5004 RTP/AVP 111\r\n\
                    a=rtpmap:111 opus/48000/2\r\n\
                    a=rtpmap:0 PCMU/8000\r\n";
        let (name, pt, rate) = extract_first_codec(sdp).unwrap();
        assert_eq!(name, "opus");
        assert_eq!(pt, 111);
        assert_eq!(rate, 48000);
    }

    #[test]
    fn test_extract_first_codec_none() {
        let sdp = "v=0\r\nm=audio 5004 RTP/AVP 0\r\n";
        assert!(extract_first_codec(sdp).is_none());
    }

    // --- Selective recording rule matching ---

    fn make_rule(
        caller: Option<&str>,
        callee: Option<&str>,
        trunks: Option<Vec<&str>>,
        direction: Option<CallDirection>,
    ) -> CompiledRecordingRule {
        CompiledRecordingRule {
            name: "test-rule".into(),
            caller_regex: caller.map(|p| Regex::new(p).unwrap()),
            callee_regex: callee.map(|p| Regex::new(p).unwrap()),
            trunk_names: trunks.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            direction,
        }
    }

    #[test]
    fn test_rule_matches_all_wildcards() {
        let rule = make_rule(None, None, None, None);
        assert!(rule_matches(
            &rule,
            "sip:alice@example.com",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            Some("trunk-1"),
        ));
    }

    #[test]
    fn test_rule_matches_direction() {
        let rule = make_rule(None, None, None, Some(CallDirection::Inbound));
        assert!(rule_matches(
            &rule,
            "sip:alice@example.com",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            None,
        ));
        assert!(!rule_matches(
            &rule,
            "sip:alice@example.com",
            "sip:bob@example.com",
            &CallDirection::Outbound,
            None,
        ));
    }

    #[test]
    fn test_rule_matches_trunk() {
        let rule = make_rule(None, None, Some(vec!["carrier-a", "carrier-b"]), None);
        assert!(rule_matches(
            &rule,
            "sip:a@x.com",
            "sip:b@y.com",
            &CallDirection::Inbound,
            Some("carrier-a"),
        ));
        assert!(!rule_matches(
            &rule,
            "sip:a@x.com",
            "sip:b@y.com",
            &CallDirection::Inbound,
            Some("carrier-c"),
        ));
        // No trunk name when rule requires one
        assert!(!rule_matches(
            &rule,
            "sip:a@x.com",
            "sip:b@y.com",
            &CallDirection::Inbound,
            None,
        ));
    }

    #[test]
    fn test_rule_matches_caller_pattern() {
        let rule = make_rule(Some(r"^\+1"), None, None, None);
        assert!(rule_matches(
            &rule,
            "+15551234567",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            None,
        ));
        assert!(!rule_matches(
            &rule,
            "+442071234567",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            None,
        ));
    }

    #[test]
    fn test_rule_matches_callee_pattern() {
        let rule = make_rule(None, Some(r"@sales\."), None, None);
        assert!(rule_matches(
            &rule,
            "sip:alice@example.com",
            "sip:bob@sales.example.com",
            &CallDirection::Inbound,
            None,
        ));
        assert!(!rule_matches(
            &rule,
            "sip:alice@example.com",
            "sip:bob@support.example.com",
            &CallDirection::Inbound,
            None,
        ));
    }

    #[test]
    fn test_rule_matches_combined_filters() {
        let rule = make_rule(
            Some(r"^\+1"),
            None,
            Some(vec!["carrier-a"]),
            Some(CallDirection::Inbound),
        );
        // All match
        assert!(rule_matches(
            &rule,
            "+15551234567",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            Some("carrier-a"),
        ));
        // Wrong direction
        assert!(!rule_matches(
            &rule,
            "+15551234567",
            "sip:bob@example.com",
            &CallDirection::Outbound,
            Some("carrier-a"),
        ));
        // Wrong trunk
        assert!(!rule_matches(
            &rule,
            "+15551234567",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            Some("carrier-b"),
        ));
        // Wrong caller pattern
        assert!(!rule_matches(
            &rule,
            "+442071234567",
            "sip:bob@example.com",
            &CallDirection::Inbound,
            Some("carrier-a"),
        ));
    }
}
