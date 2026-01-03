//! SDP conversion utilities
//!
//! Converts between sip-sdp and forge-media SDP formats

use crate::{Error, Result};
use std::collections::HashMap;

/// Convert SIP SDP to forge-media SDP format
///
/// This is a placeholder - actual implementation will depend on
/// forge-media's SDP representation
pub fn convert_sip_to_forge_sdp(sip_sdp: &str) -> Result<String> {
    // For now, pass through as string
    // Future: Parse with sip-sdp, convert to forge format
    Ok(sip_sdp.to_string())
}

/// Convert forge-media SDP to SIP SDP format
pub fn convert_forge_to_sip_sdp(forge_sdp: &str) -> Result<String> {
    // For now, pass through as string
    // Future: Convert from forge format to SIP SDP string
    Ok(forge_sdp.to_string())
}

/// Rewrite SDP to point to server's RTP endpoint
///
/// This function modifies the SDP to redirect RTP traffic through the server
/// by replacing the connection address and media port.
///
/// # Arguments
/// * `original_sdp` - The original SDP body
/// * `server_ip` - The server's IP address for RTP
/// * `rtp_port` - The RTP port allocated by forge-media
///
/// # Returns
/// Modified SDP string with updated c= and m= lines
///
/// # Example
/// ```ignore
/// let original = "v=0\no=- 123 456 IN IP4 192.168.1.100\n...";
/// let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000);
/// // Result: c= line changed to 10.0.0.5, m= line port changed to 20000
/// ```
pub fn rewrite_sdp_for_anchoring(original_sdp: &str, server_ip: &str, rtp_port: u16) -> String {
    let mut result = String::new();
    let mut in_audio_section = false;
    let mut audio_connection_replaced = false;

    for line in original_sdp.lines() {
        let trimmed = line.trim();

        // Track if we're in the audio media section
        if trimmed.starts_with("m=") {
            in_audio_section = trimmed.starts_with("m=audio");
            audio_connection_replaced = false; // Reset for each media section
        }

        // Replace connection address (c=)
        // Can be at session level or media level
        if trimmed.starts_with("c=IN IP") {
            // Only replace if we're in audio section or it's session-level (before any m=)
            if in_audio_section || !audio_connection_replaced {
                // Parse: c=IN IP4 192.168.1.100 or c=IN IP6 ::1
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 3 {
                    let ip_version = parts[1]; // IP4 or IP6
                    result.push_str(&format!("c=IN {} {}\n", ip_version, server_ip));
                    audio_connection_replaced = true;
                    continue;
                }
            }
        }

        // Replace audio media port (m=audio)
        if trimmed.starts_with("m=audio ") {
            // Parse: m=audio 10000 RTP/AVP 0 8 101
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 3 {
                // Reconstruct with new port
                result.push_str("m=audio ");
                result.push_str(&rtp_port.to_string());
                // Keep the rest of the line (protocol and codecs)
                for part in &parts[2..] {
                    result.push(' ');
                    result.push_str(part);
                }
                result.push('\n');
                continue;
            }
        }

        // Keep all other lines unchanged
        result.push_str(line);
        result.push('\n');
    }

    result
}

/// Extract audio codecs from SDP body
///
/// Parses SDP to extract codec names from rtpmap attributes.
/// Example SDP:
/// ```text
/// m=audio 10000 RTP/AVP 0 8 101
/// a=rtpmap:0 PCMU/8000
/// a=rtpmap:8 PCMA/8000
/// a=rtpmap:101 telephone-event/8000
/// ```
///
/// Returns: ["PCMU", "PCMA", "telephone-event"]
pub fn extract_codecs_from_sdp(sdp_body: &str) -> Vec<String> {
    let mut codecs = Vec::new();
    let mut payload_types: Vec<String> = Vec::new();
    let mut rtpmap: HashMap<String, String> = HashMap::new();

    for line in sdp_body.lines() {
        let line = line.trim();

        // Parse media line: m=audio 10000 RTP/AVP 0 8 101
        if line.starts_with("m=audio") {
            // Extract payload types from m= line
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 3 {
                // Skip "m=audio", port, and protocol, rest are payload types
                payload_types = parts[3..].iter().map(|s| s.to_string()).collect();
            }
        }

        // Parse rtpmap: a=rtpmap:0 PCMU/8000
        if line.starts_with("a=rtpmap:") {
            let rtpmap_data = &line[9..]; // Skip "a=rtpmap:"
            if let Some((pt, codec_info)) = rtpmap_data.split_once(' ') {
                // Extract codec name (before the slash)
                let codec_name = codec_info.split('/').next().unwrap_or(codec_info);
                rtpmap.insert(pt.to_string(), codec_name.to_string());
            }
        }
    }

    // Match payload types with codec names from rtpmap
    for pt in &payload_types {
        if let Some(codec) = rtpmap.get(pt) {
            // Skip telephone-event and other non-audio codecs
            if !codec.eq_ignore_ascii_case("telephone-event") {
                codecs.push(codec.clone());
            }
        }
    }

    codecs
}

/// Negotiate codecs between offer and answer
pub fn negotiate_codecs(offer: &[String], answer: &[String]) -> Result<Vec<String>> {
    let mut negotiated = Vec::new();

    for codec in offer {
        if answer.iter().any(|c| c.eq_ignore_ascii_case(codec)) {
            negotiated.push(codec.clone());
        }
    }

    if negotiated.is_empty() {
        return Err(Error::CodecNegotiation(
            "No common codecs found".to_string(),
        ));
    }

    Ok(negotiated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_sdp_for_anchoring() {
        let original = r#"v=0
o=- 123456 123456 IN IP4 192.168.1.100
s=Test Call
c=IN IP4 192.168.1.100
t=0 0
m=audio 10000 RTP/AVP 0 8 101
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000
a=rtpmap:101 telephone-event/8000
"#;

        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000);

        // Check that connection address was replaced
        assert!(rewritten.contains("c=IN IP4 10.0.0.5"));
        assert!(!rewritten.contains("c=IN IP4 192.168.1.100"));

        // Check that media port was replaced
        assert!(rewritten.contains("m=audio 20000 RTP/AVP"));
        assert!(!rewritten.contains("m=audio 10000 RTP/AVP"));

        // Check that codecs are preserved
        assert!(rewritten.contains("a=rtpmap:0 PCMU/8000"));
        assert!(rewritten.contains("a=rtpmap:8 PCMA/8000"));
    }

    #[test]
    fn test_rewrite_sdp_media_level_connection() {
        let original = r#"v=0
o=- 123456 123456 IN IP4 192.168.1.100
s=Test
t=0 0
m=audio 10000 RTP/AVP 0
c=IN IP4 192.168.1.100
a=rtpmap:0 PCMU/8000
"#;

        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000);

        assert!(rewritten.contains("c=IN IP4 10.0.0.5"));
        assert!(rewritten.contains("m=audio 20000 RTP/AVP"));
    }

    #[test]
    fn test_rewrite_sdp_ipv6() {
        let original = r#"v=0
o=- 123456 123456 IN IP6 ::1
s=Test
c=IN IP6 2001:db8::1
t=0 0
m=audio 10000 RTP/AVP 0
a=rtpmap:0 PCMU/8000
"#;

        let rewritten = rewrite_sdp_for_anchoring(original, "2001:db8::5", 20000);

        assert!(rewritten.contains("c=IN IP6 2001:db8::5"));
        assert!(rewritten.contains("m=audio 20000 RTP/AVP"));
    }

    #[test]
    fn test_extract_codecs_from_sdp() {
        let sdp = r#"v=0
o=- 123456 123456 IN IP4 192.168.1.100
s=Test
c=IN IP4 192.168.1.100
t=0 0
m=audio 10000 RTP/AVP 0 8 101
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000
a=rtpmap:101 telephone-event/8000
a=ptime:20
"#;

        let codecs = extract_codecs_from_sdp(sdp);
        assert_eq!(codecs.len(), 2);
        assert_eq!(codecs[0], "PCMU");
        assert_eq!(codecs[1], "PCMA");
        // telephone-event should be filtered out
    }

    #[test]
    fn test_extract_codecs_opus() {
        let sdp = r#"v=0
o=- 123456 123456 IN IP4 192.168.1.100
s=Test
c=IN IP4 192.168.1.100
t=0 0
m=audio 10000 RTP/AVP 111 0
a=rtpmap:111 opus/48000/2
a=rtpmap:0 PCMU/8000
"#;

        let codecs = extract_codecs_from_sdp(sdp);
        assert_eq!(codecs.len(), 2);
        assert_eq!(codecs[0], "opus");
        assert_eq!(codecs[1], "PCMU");
    }

    #[test]
    fn test_extract_codecs_empty() {
        let sdp = "v=0\ns=Test\n";
        let codecs = extract_codecs_from_sdp(sdp);
        assert_eq!(codecs.len(), 0);
    }

    #[test]
    fn test_codec_negotiation() {
        let offer = vec!["PCMU".to_string(), "opus".to_string(), "G722".to_string()];
        let answer = vec!["PCMA".to_string(), "opus".to_string()];

        let result = negotiate_codecs(&offer, &answer).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "opus");
    }

    #[test]
    fn test_codec_negotiation_no_match() {
        let offer = vec!["PCMU".to_string()];
        let answer = vec!["opus".to_string()];

        let result = negotiate_codecs(&offer, &answer);
        assert!(result.is_err());
    }

    #[test]
    fn test_codec_negotiation_case_insensitive() {
        let offer = vec!["pcmu".to_string()];
        let answer = vec!["PCMU".to_string()];

        let result = negotiate_codecs(&offer, &answer).unwrap();
        assert_eq!(result.len(), 1);
    }
}
