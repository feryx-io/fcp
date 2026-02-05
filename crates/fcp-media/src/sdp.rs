//! SDP conversion utilities
//!
//! Converts between sip-sdp and forge-media SDP formats

use base64::{engine::general_purpose, Engine as _};
use crate::{Error, Result};
use forge_rtp::srtp::SrtpProfile;
use std::collections::HashMap;

/// Structured codec info extracted from SDP
#[derive(Debug, Clone)]
pub struct SdpCodecInfo {
    /// RTP payload type number (0-127)
    pub payload_type: u8,
    /// Encoding name as it appears in SDP (e.g., "PCMU", "PCMA", "G722", "opus")
    pub encoding_name: String,
    /// Clock rate in Hz (e.g., 8000, 16000, 48000)
    pub clock_rate: u32,
}

/// Parsed SDES crypto line from SDP (RFC 4568).
///
/// Format: `a=crypto:<tag> <suite> <key-params> [session-params]`
#[derive(Debug, Clone)]
pub struct SdesCryptoLine {
    /// Crypto tag (1-9 digits, identifies the crypto attribute)
    pub tag: u32,
    /// Crypto suite name (e.g., "AES_CM_128_HMAC_SHA1_80")
    pub suite: String,
    /// Key parameters (e.g., "inline:base64key|...")
    pub key_params: String,
}

/// Parse audio codecs from SDP with payload types and clock rates.
///
/// Uses `sip_sdp::SessionDescription::parse()` for robust parsing, with a
/// manual fallback for malformed SDP. Filters out telephone-event entries.
pub fn parse_sdp_codecs(sdp_body: &str) -> Vec<SdpCodecInfo> {
    // Try sip_sdp parser first
    if let Ok(sdp) = sip_sdp::SessionDescription::parse(sdp_body) {
        if let Some(audio) = sdp.find_media(sip_sdp::MediaType::Audio) {
            let mut codecs = Vec::new();
            for fmt in &audio.formats {
                if let Ok(pt) = fmt.as_str().parse::<u8>() {
                    if let Some(rtpmap) = audio.rtpmaps.get(&pt) {
                        if !rtpmap
                            .encoding_name
                            .as_str()
                            .eq_ignore_ascii_case("telephone-event")
                        {
                            codecs.push(SdpCodecInfo {
                                payload_type: pt,
                                encoding_name: rtpmap.encoding_name.to_string(),
                                clock_rate: rtpmap.clock_rate,
                            });
                        }
                    } else if pt < 96 {
                        // Static PT without rtpmap — infer codec
                        if let Some((name, rate)) = static_pt_info(pt) {
                            codecs.push(SdpCodecInfo {
                                payload_type: pt,
                                encoding_name: name.to_string(),
                                clock_rate: rate,
                            });
                        }
                    }
                }
            }
            return codecs;
        }
    }
    // Fallback: manual parsing (tolerant of malformed SDP)
    parse_sdp_codecs_manual(sdp_body)
}

/// Extract telephone-event payload type from SDP. Returns `None` if absent.
pub fn extract_telephone_event_pt(sdp_body: &str) -> Option<u8> {
    // Try sip_sdp parser first
    if let Ok(sdp) = sip_sdp::SessionDescription::parse(sdp_body) {
        if let Some(audio) = sdp.find_media(sip_sdp::MediaType::Audio) {
            for (pt, rtpmap) in &audio.rtpmaps {
                if rtpmap
                    .encoding_name
                    .as_str()
                    .eq_ignore_ascii_case("telephone-event")
                {
                    return Some(*pt);
                }
            }
        }
        return None;
    }
    // Fallback: manual scan
    for line in sdp_body.lines() {
        let line = line.trim();
        if line.starts_with("a=rtpmap:") {
            let rtpmap_data = &line[9..];
            if let Some((pt_str, codec_info)) = rtpmap_data.split_once(' ') {
                let codec_name = codec_info.split('/').next().unwrap_or(codec_info);
                if codec_name.eq_ignore_ascii_case("telephone-event") {
                    if let Ok(pt) = pt_str.parse::<u8>() {
                        return Some(pt);
                    }
                }
            }
        }
    }
    None
}

/// Map well-known static payload types to codec info.
fn static_pt_info(pt: u8) -> Option<(&'static str, u32)> {
    match pt {
        0 => Some(("PCMU", 8000)),
        8 => Some(("PCMA", 8000)),
        9 => Some(("G722", 16000)),
        18 => Some(("G729", 8000)),
        _ => None,
    }
}

/// Manual fallback parser for malformed SDP — returns `SdpCodecInfo` entries.
fn parse_sdp_codecs_manual(sdp_body: &str) -> Vec<SdpCodecInfo> {
    let mut payload_types: Vec<String> = Vec::new();
    let mut rtpmap_info: HashMap<String, (String, u32)> = HashMap::new();

    for line in sdp_body.lines() {
        let line = line.trim();

        if line.starts_with("m=audio") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 3 {
                payload_types = parts[3..].iter().map(|s| s.to_string()).collect();
            }
        }

        if line.starts_with("a=rtpmap:") {
            let rtpmap_data = &line[9..];
            if let Some((pt, codec_info)) = rtpmap_data.split_once(' ') {
                let parts: Vec<&str> = codec_info.split('/').collect();
                let codec_name = parts[0];
                let clock_rate = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(8000);
                rtpmap_info.insert(pt.to_string(), (codec_name.to_string(), clock_rate));
            }
        }
    }

    let mut codecs = Vec::new();
    for pt_str in &payload_types {
        if let Ok(pt) = pt_str.parse::<u8>() {
            if let Some((name, rate)) = rtpmap_info.get(pt_str) {
                if !name.eq_ignore_ascii_case("telephone-event") {
                    codecs.push(SdpCodecInfo {
                        payload_type: pt,
                        encoding_name: name.clone(),
                        clock_rate: *rate,
                    });
                }
            } else if pt < 96 {
                if let Some((name, rate)) = static_pt_info(pt) {
                    codecs.push(SdpCodecInfo {
                        payload_type: pt,
                        encoding_name: name.to_string(),
                        clock_rate: rate,
                    });
                }
            }
        }
    }

    codecs
}

/// Parse all `a=crypto:` lines from an SDP body (RFC 4568).
///
/// Uses the `sip_sdp` parser first to extract media-level attributes, falling
/// back to manual line scanning for malformed SDP.  Returns an empty vec if no
/// crypto lines are found (plain RTP).
pub fn parse_sdes_crypto(sdp_body: &str) -> Vec<SdesCryptoLine> {
    // Try sip_sdp parser first (media-level attributes)
    if let Ok(sdp) = sip_sdp::SessionDescription::parse(sdp_body) {
        if let Some(audio) = sdp.find_media(sip_sdp::MediaType::Audio) {
            let mut lines = Vec::new();
            for attr in &audio.attributes {
                if let sip_sdp::Attribute::Value { name, value } = attr {
                    if name.as_str() == "crypto" {
                        if let Some(parsed) = parse_crypto_value(value.as_str()) {
                            lines.push(parsed);
                        }
                    }
                }
            }
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    // Fallback: manual scan
    parse_sdes_crypto_manual(sdp_body)
}

/// Parse a single crypto attribute value.
///
/// Format: `<tag> <suite> <key-params> [session-params]`
fn parse_crypto_value(value: &str) -> Option<SdesCryptoLine> {
    let parts: Vec<&str> = value.splitn(4, ' ').collect();
    if parts.len() >= 3 {
        Some(SdesCryptoLine {
            tag: parts[0].parse().ok()?,
            suite: parts[1].to_string(),
            key_params: parts[2].to_string(),
        })
    } else {
        None
    }
}

/// Manual fallback for SDES crypto parsing from raw SDP lines.
fn parse_sdes_crypto_manual(sdp_body: &str) -> Vec<SdesCryptoLine> {
    sdp_body
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("a=crypto:")
                .and_then(parse_crypto_value)
        })
        .collect()
}

/// Decode SDES inline key params to raw key+salt bytes.
///
/// Format: `"inline:base64(key||salt)[|lifetime][|MKI]"`
///
/// The `suite` parameter is used to determine expected key/salt lengths.
/// Returns `(master_key, master_salt, profile)`.
pub fn decode_sdes_key_params(
    key_params: &str,
    suite: &str,
) -> Result<(Vec<u8>, Vec<u8>, SrtpProfile)> {
    let b64_part = key_params
        .strip_prefix("inline:")
        .ok_or_else(|| Error::Sdp("SDES key_params must start with 'inline:'".into()))?;
    // Strip lifetime/MKI if present (separated by |)
    let b64_key = b64_part.split('|').next().unwrap_or(b64_part);
    let decoded = general_purpose::STANDARD
        .decode(b64_key)
        .map_err(|e| Error::Sdp(format!("Failed to decode SDES key: {}", e)))?;

    let profile = sdes_suite_to_profile(suite)
        .ok_or_else(|| Error::Sdp(format!("Unsupported SDES suite: {}", suite)))?;

    let key_len = profile.master_key_len();
    let salt_len = profile.master_salt_len();
    let expected = key_len + salt_len;

    if decoded.len() != expected {
        return Err(Error::Sdp(format!(
            "SDES key material length mismatch for {}: expected {} bytes, got {}",
            suite, expected, decoded.len()
        )));
    }

    Ok((
        decoded[..key_len].to_vec(),
        decoded[key_len..].to_vec(),
        profile,
    ))
}

/// Map SDES crypto suite name to forge `SrtpProfile`.
pub fn sdes_suite_to_profile(suite: &str) -> Option<SrtpProfile> {
    match suite {
        "AES_CM_128_HMAC_SHA1_80" => Some(SrtpProfile::Aes128CmHmacSha1_80),
        "AES_CM_128_HMAC_SHA1_32" => Some(SrtpProfile::Aes128CmHmacSha1_32),
        "AEAD_AES_128_GCM" => Some(SrtpProfile::AeadAes128Gcm),
        "AEAD_AES_256_GCM" => Some(SrtpProfile::AeadAes256Gcm),
        _ => None,
    }
}

/// Generate random SRTP key material for AES_CM_128_HMAC_SHA1_80.
///
/// Returns `(SrtpKeyMaterial, base64_key_params_string)` suitable for
/// embedding in an SDP `a=crypto:` line.
pub fn generate_srtp_key_material() -> Result<(forge_rtp::srtp::SrtpKeyMaterial, String)> {
    use rand::RngCore;

    let profile = SrtpProfile::Aes128CmHmacSha1_80;
    let mut key = vec![0u8; profile.master_key_len()];
    let mut salt = vec![0u8; profile.master_salt_len()];
    rand::thread_rng().fill_bytes(&mut key);
    rand::thread_rng().fill_bytes(&mut salt);

    // Concatenate key||salt and base64 encode for SDP
    let mut combined = Vec::with_capacity(key.len() + salt.len());
    combined.extend_from_slice(&key);
    combined.extend_from_slice(&salt);
    let b64 = general_purpose::STANDARD.encode(&combined);

    let material = forge_rtp::srtp::SrtpKeyMaterial::new(key, salt, profile)
        .map_err(|e| Error::ForgeMedia(e.to_string()))?;

    Ok((material, b64))
}

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
                    result.push_str(&format!("c=IN {} {}\r\n", ip_version, server_ip));
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
                result.push_str("\r\n");
                continue;
            }
        }

        // Keep all other lines unchanged
        result.push_str(line);
        result.push_str("\r\n");
    }

    result
}

/// Filter SDP to only include specified codecs.
///
/// Used by B2BUA to constrain B-leg offer to codecs A-leg supports,
/// ensuring codec alignment or controlled transcoding.
///
/// # Arguments
/// * `original_sdp` - The original SDP body
/// * `allowed_codecs` - List of allowed codec names (e.g., ["G722", "PCMU"])
///
/// # Returns
/// Filtered SDP string with only allowed codecs in m= and a=rtpmap lines.
/// Always preserves telephone-event for DTMF.
pub fn filter_sdp_codecs(original_sdp: &str, allowed_codecs: &[&str]) -> String {
    // Use existing parse_sdp_codecs() to get codec info
    let codecs = parse_sdp_codecs(original_sdp);

    // Build list of allowed payload types
    let mut allowed_pts: Vec<u8> = codecs
        .iter()
        .filter(|c| {
            allowed_codecs
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&c.encoding_name))
        })
        .map(|c| c.payload_type)
        .collect();

    // Always include telephone-event if present
    if let Some(te_pt) = extract_telephone_event_pt(original_sdp) {
        if !allowed_pts.contains(&te_pt) {
            allowed_pts.push(te_pt);
        }
    }

    // Rebuild SDP line by line
    let mut result = String::new();
    for line in original_sdp.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("m=audio ") {
            // Filter m=audio payload types
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 4 {
                result.push_str(&format!("m=audio {} {}", parts[1], parts[2]));
                for pt in parts[3..].iter() {
                    if let Ok(pt_num) = pt.parse::<u8>() {
                        if allowed_pts.contains(&pt_num) {
                            result.push(' ');
                            result.push_str(pt);
                        }
                    }
                }
                result.push_str("\r\n");
                continue;
            }
        }

        // Filter a=rtpmap and a=fmtp lines
        if trimmed.starts_with("a=rtpmap:") || trimmed.starts_with("a=fmtp:") {
            let attr_data = if trimmed.starts_with("a=rtpmap:") {
                &trimmed[9..]
            } else {
                &trimmed[7..]
            };
            if let Some(pt_str) = attr_data
                .split_whitespace()
                .next()
                .or_else(|| attr_data.split(':').next())
            {
                if let Ok(pt) = pt_str.parse::<u8>() {
                    if !allowed_pts.contains(&pt) {
                        continue; // Skip this line
                    }
                }
            }
        }

        result.push_str(line);
        result.push_str("\r\n");
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

    #[test]
    fn test_parse_sdp_codecs_basic() {
        let sdp = "v=0\r\n\
                    o=- 123 1 IN IP4 192.168.1.100\r\n\
                    s=Test\r\n\
                    c=IN IP4 192.168.1.100\r\n\
                    t=0 0\r\n\
                    m=audio 10000 RTP/AVP 0 8 101\r\n\
                    a=rtpmap:0 PCMU/8000\r\n\
                    a=rtpmap:8 PCMA/8000\r\n\
                    a=rtpmap:101 telephone-event/8000\r\n\
                    a=fmtp:101 0-15\r\n";

        let codecs = parse_sdp_codecs(sdp);
        assert_eq!(codecs.len(), 2);
        assert_eq!(codecs[0].encoding_name, "PCMU");
        assert_eq!(codecs[0].payload_type, 0);
        assert_eq!(codecs[0].clock_rate, 8000);
        assert_eq!(codecs[1].encoding_name, "PCMA");
        assert_eq!(codecs[1].payload_type, 8);
        assert_eq!(codecs[1].clock_rate, 8000);
    }

    #[test]
    fn test_parse_sdp_codecs_dynamic() {
        let sdp = "v=0\r\n\
                    o=- 123 1 IN IP4 10.0.0.1\r\n\
                    s=Test\r\n\
                    c=IN IP4 10.0.0.1\r\n\
                    t=0 0\r\n\
                    m=audio 5000 RTP/AVP 9 111\r\n\
                    a=rtpmap:9 G722/16000\r\n\
                    a=rtpmap:111 opus/48000/2\r\n";

        let codecs = parse_sdp_codecs(sdp);
        assert_eq!(codecs.len(), 2);
        assert_eq!(codecs[0].encoding_name, "G722");
        assert_eq!(codecs[0].payload_type, 9);
        assert_eq!(codecs[0].clock_rate, 16000);
        assert_eq!(codecs[1].encoding_name, "opus");
        assert_eq!(codecs[1].payload_type, 111);
        assert_eq!(codecs[1].clock_rate, 48000);
    }

    #[test]
    fn test_parse_sdp_codecs_static_no_rtpmap() {
        let sdp = "v=0\r\n\
                    o=- 123 1 IN IP4 10.0.0.1\r\n\
                    s=Test\r\n\
                    c=IN IP4 10.0.0.1\r\n\
                    t=0 0\r\n\
                    m=audio 5000 RTP/AVP 0 8\r\n";

        let codecs = parse_sdp_codecs(sdp);
        assert_eq!(codecs.len(), 2);
        assert_eq!(codecs[0].encoding_name, "PCMU");
        assert_eq!(codecs[0].payload_type, 0);
        assert_eq!(codecs[0].clock_rate, 8000);
        assert_eq!(codecs[1].encoding_name, "PCMA");
        assert_eq!(codecs[1].payload_type, 8);
        assert_eq!(codecs[1].clock_rate, 8000);
    }

    #[test]
    fn test_extract_telephone_event_pt() {
        let sdp_with = "v=0\r\n\
                         o=- 123 1 IN IP4 10.0.0.1\r\n\
                         s=Test\r\n\
                         c=IN IP4 10.0.0.1\r\n\
                         t=0 0\r\n\
                         m=audio 5000 RTP/AVP 0 101\r\n\
                         a=rtpmap:0 PCMU/8000\r\n\
                         a=rtpmap:101 telephone-event/8000\r\n";

        assert_eq!(extract_telephone_event_pt(sdp_with), Some(101));

        let sdp_without = "v=0\r\n\
                            o=- 123 1 IN IP4 10.0.0.1\r\n\
                            s=Test\r\n\
                            c=IN IP4 10.0.0.1\r\n\
                            t=0 0\r\n\
                            m=audio 5000 RTP/AVP 0\r\n\
                            a=rtpmap:0 PCMU/8000\r\n";

        assert_eq!(extract_telephone_event_pt(sdp_without), None);
    }

    #[test]
    fn test_parse_sdes_crypto_basic() {
        let sdp = "v=0\r\n\
                    o=- 123 1 IN IP4 10.0.0.1\r\n\
                    s=Test\r\n\
                    c=IN IP4 10.0.0.1\r\n\
                    t=0 0\r\n\
                    m=audio 5000 RTP/SAVP 0\r\n\
                    a=rtpmap:0 PCMU/8000\r\n\
                    a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:dGVzdGtleW1hdGVyaWFsMTIzNDU2Nzg5MA==\r\n";

        let crypto = parse_sdes_crypto(sdp);
        assert_eq!(crypto.len(), 1);
        assert_eq!(crypto[0].tag, 1);
        assert_eq!(crypto[0].suite, "AES_CM_128_HMAC_SHA1_80");
        assert!(crypto[0].key_params.starts_with("inline:"));
    }

    #[test]
    fn test_parse_sdes_crypto_multiple() {
        let sdp = "v=0\r\n\
                    o=- 123 1 IN IP4 10.0.0.1\r\n\
                    s=Test\r\n\
                    c=IN IP4 10.0.0.1\r\n\
                    t=0 0\r\n\
                    m=audio 5000 RTP/SAVP 0\r\n\
                    a=rtpmap:0 PCMU/8000\r\n\
                    a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:aWZpdGtleW1hdGVyaWFsMTIzNDU2Nzg5MA==\r\n\
                    a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:c2Vjb25ka2V5bWF0ZXJpYWwxMjM0NTY3OA==\r\n";

        let crypto = parse_sdes_crypto(sdp);
        assert_eq!(crypto.len(), 2);
        assert_eq!(crypto[0].tag, 1);
        assert_eq!(crypto[0].suite, "AES_CM_128_HMAC_SHA1_80");
        assert_eq!(crypto[1].tag, 2);
        assert_eq!(crypto[1].suite, "AES_CM_128_HMAC_SHA1_32");
    }

    #[test]
    fn test_parse_sdes_crypto_none() {
        let sdp = "v=0\r\n\
                    o=- 123 1 IN IP4 10.0.0.1\r\n\
                    s=Test\r\n\
                    c=IN IP4 10.0.0.1\r\n\
                    t=0 0\r\n\
                    m=audio 5000 RTP/AVP 0\r\n\
                    a=rtpmap:0 PCMU/8000\r\n";

        let crypto = parse_sdes_crypto(sdp);
        assert!(crypto.is_empty());
    }

    #[test]
    fn test_decode_sdes_key_params() {
        // 16-byte key + 14-byte salt = 30 bytes for AES_CM_128
        let key_bytes = [0x01u8; 16];
        let salt_bytes = [0x02u8; 14];
        let mut combined = Vec::new();
        combined.extend_from_slice(&key_bytes);
        combined.extend_from_slice(&salt_bytes);
        let b64 = general_purpose::STANDARD.encode(&combined);
        let key_params = format!("inline:{}", b64);

        let (key, salt, profile) =
            decode_sdes_key_params(&key_params, "AES_CM_128_HMAC_SHA1_80").unwrap();
        assert_eq!(key.len(), 16);
        assert_eq!(salt.len(), 14);
        assert_eq!(key, key_bytes);
        assert_eq!(salt, salt_bytes);
        assert_eq!(profile, SrtpProfile::Aes128CmHmacSha1_80);
    }

    #[test]
    fn test_decode_sdes_key_params_with_lifetime() {
        let key_bytes = [0xAAu8; 16];
        let salt_bytes = [0xBBu8; 14];
        let mut combined = Vec::new();
        combined.extend_from_slice(&key_bytes);
        combined.extend_from_slice(&salt_bytes);
        let b64 = general_purpose::STANDARD.encode(&combined);
        // Lifetime suffix after |
        let key_params = format!("inline:{}|2^31", b64);

        let (key, salt, _profile) =
            decode_sdes_key_params(&key_params, "AES_CM_128_HMAC_SHA1_80").unwrap();
        assert_eq!(key, key_bytes);
        assert_eq!(salt, salt_bytes);
    }

    #[test]
    fn test_decode_sdes_key_params_bad_prefix() {
        let result = decode_sdes_key_params("notinline:abc", "AES_CM_128_HMAC_SHA1_80");
        assert!(result.is_err());
    }

    #[test]
    fn test_sdes_suite_to_profile() {
        assert_eq!(
            sdes_suite_to_profile("AES_CM_128_HMAC_SHA1_80"),
            Some(SrtpProfile::Aes128CmHmacSha1_80)
        );
        assert_eq!(
            sdes_suite_to_profile("AES_CM_128_HMAC_SHA1_32"),
            Some(SrtpProfile::Aes128CmHmacSha1_32)
        );
        assert_eq!(
            sdes_suite_to_profile("AEAD_AES_128_GCM"),
            Some(SrtpProfile::AeadAes128Gcm)
        );
        assert_eq!(
            sdes_suite_to_profile("AEAD_AES_256_GCM"),
            Some(SrtpProfile::AeadAes256Gcm)
        );
        assert_eq!(sdes_suite_to_profile("UNKNOWN_SUITE"), None);
    }

    #[test]
    fn test_generate_srtp_key_material() {
        let (material, b64) = generate_srtp_key_material().unwrap();
        assert_eq!(material.master_key.len(), 16);
        assert_eq!(material.master_salt.len(), 14);
        assert_eq!(material.profile, SrtpProfile::Aes128CmHmacSha1_80);
        // Base64 of 30 bytes = 40 chars
        assert_eq!(b64.len(), 40);

        // Verify round-trip: decode the base64 back
        let decoded = general_purpose::STANDARD.decode(&b64).unwrap();
        assert_eq!(decoded.len(), 30);
        assert_eq!(&decoded[..16], &material.master_key[..]);
        assert_eq!(&decoded[16..], &material.master_salt[..]);
    }

    #[test]
    fn test_filter_sdp_codecs_basic() {
        let sdp = "v=0\r\n\
                   o=- 123 1 IN IP4 192.168.1.100\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 9 0 8 101\r\n\
                   a=rtpmap:9 G722/8000\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=rtpmap:8 PCMA/8000\r\n\
                   a=rtpmap:101 telephone-event/8000\r\n\
                   a=fmtp:101 0-15\r\n";

        // Filter to only G722
        let filtered = filter_sdp_codecs(sdp, &["G722"]);

        // Should have G722 and telephone-event only
        assert!(filtered.contains("m=audio 10000 RTP/AVP 9 101"));
        assert!(filtered.contains("a=rtpmap:9 G722/8000"));
        assert!(filtered.contains("a=rtpmap:101 telephone-event/8000"));
        assert!(filtered.contains("a=fmtp:101 0-15"));
        // Should NOT have PCMU or PCMA
        assert!(!filtered.contains("a=rtpmap:0 PCMU"));
        assert!(!filtered.contains("a=rtpmap:8 PCMA"));
    }

    #[test]
    fn test_filter_sdp_codecs_with_fallback() {
        let sdp = "v=0\r\n\
                   o=- 123 1 IN IP4 192.168.1.100\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 9 0 8 101\r\n\
                   a=rtpmap:9 G722/8000\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=rtpmap:8 PCMA/8000\r\n\
                   a=rtpmap:101 telephone-event/8000\r\n";

        // Filter to G722 + PCMU fallback
        let filtered = filter_sdp_codecs(sdp, &["G722", "PCMU"]);

        // Should have G722, PCMU, and telephone-event
        assert!(filtered.contains("m=audio 10000 RTP/AVP 9 0 101"));
        assert!(filtered.contains("a=rtpmap:9 G722/8000"));
        assert!(filtered.contains("a=rtpmap:0 PCMU/8000"));
        assert!(filtered.contains("a=rtpmap:101 telephone-event/8000"));
        // Should NOT have PCMA
        assert!(!filtered.contains("a=rtpmap:8 PCMA"));
    }

    #[test]
    fn test_filter_sdp_codecs_case_insensitive() {
        let sdp = "v=0\r\n\
                   o=- 123 1 IN IP4 192.168.1.100\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0 8\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=rtpmap:8 PCMA/8000\r\n";

        // Filter with lowercase
        let filtered = filter_sdp_codecs(sdp, &["pcmu"]);

        assert!(filtered.contains("m=audio 10000 RTP/AVP 0"));
        assert!(filtered.contains("a=rtpmap:0 PCMU/8000"));
        assert!(!filtered.contains("a=rtpmap:8 PCMA"));
    }

    #[test]
    fn test_filter_sdp_codecs_preserves_other_lines() {
        let sdp = "v=0\r\n\
                   o=- 123 1 IN IP4 192.168.1.100\r\n\
                   s=Test Call\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=ptime:20\r\n\
                   a=sendrecv\r\n";

        let filtered = filter_sdp_codecs(sdp, &["PCMU"]);

        // All non-codec lines should be preserved
        assert!(filtered.contains("v=0"));
        assert!(filtered.contains("o=- 123 1 IN IP4 192.168.1.100"));
        assert!(filtered.contains("s=Test Call"));
        assert!(filtered.contains("c=IN IP4 192.168.1.100"));
        assert!(filtered.contains("a=ptime:20"));
        assert!(filtered.contains("a=sendrecv"));
    }
}
