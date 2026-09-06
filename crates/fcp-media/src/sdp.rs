//! SDP handling for FCP: codec and crypto extraction, answers, anchoring
//! rewrites and codec filtering, all per media section.
//!
//! Audio is what FCP mixes and relays today. An offered video section is
//! answered through forge-sdp when the caller allows it ([`VideoAnswer`])
//! and otherwise *rejected properly* — port 0 with the offer's protocol
//! and formats (RFC 3264 §6) — and any other media is always rejected.
//! An answer therefore always has one section per offered section, in
//! order, which is what an RFC 3264 peer requires.

use crate::{Error, Result};
use base64::{engine::general_purpose, Engine as _};
use forge_core::VideoCodec;
use forge_rtp::srtp::SrtpProfile;
use forge_sdp::{answer_direction, answer_video, direction_of, reject_section, VideoAttributesExt};
use sip_sdp::{Attribute, MediaDescription, MediaType, Protocol, SessionDescription};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

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

// ── Codec conversions ───────────────────────────────────────────────────

/// The forge-media codec for an FCP video codec.
pub fn video_codec_to_forge(c: fcp_core::VideoCodec) -> VideoCodec {
    match c {
        fcp_core::VideoCodec::H264 => VideoCodec::H264,
        fcp_core::VideoCodec::H265 => VideoCodec::H265,
        fcp_core::VideoCodec::Vp8 => VideoCodec::VP8,
        fcp_core::VideoCodec::Vp9 => VideoCodec::VP9,
        fcp_core::VideoCodec::Av1 => VideoCodec::AV1,
    }
}

/// The FCP codec for a forge-media video codec.
pub fn video_codec_from_forge(c: VideoCodec) -> fcp_core::VideoCodec {
    match c {
        VideoCodec::H264 => fcp_core::VideoCodec::H264,
        VideoCodec::H265 => fcp_core::VideoCodec::H265,
        VideoCodec::VP8 => fcp_core::VideoCodec::Vp8,
        VideoCodec::VP9 => fcp_core::VideoCodec::Vp9,
        VideoCodec::AV1 => fcp_core::VideoCodec::Av1,
    }
}

// ── Section scoping ─────────────────────────────────────────────────────

/// The lines of the first `m=<media>` section (its `m=` line to the next
/// `m=`), for the manual fallbacks that run when the parser rejects a body.
fn first_section_lines<'a>(sdp: &'a str, media: &str) -> Vec<&'a str> {
    let prefix = format!("m={media}");
    let mut out = Vec::new();
    let mut inside = false;
    let mut seen = false;
    for line in sdp.lines() {
        let t = line.trim();
        if t.starts_with("m=") {
            if inside {
                break;
            }
            if !seen && (t == prefix || t.starts_with(&format!("{prefix} "))) {
                inside = true;
                seen = true;
            }
        }
        if inside {
            out.push(t);
        }
    }
    out
}

/// The first section of a type, however its port is set.
fn first_section(sdp: &SessionDescription, media: MediaType) -> Option<&MediaDescription> {
    sdp.media.iter().find(|m| m.media_type == media)
}

/// The codecs of one section, in format order: rtpmap entries other than
/// telephone-event, plus well-known static types without an rtpmap.
fn codecs_of_section(m: &MediaDescription) -> Vec<SdpCodecInfo> {
    let mut codecs = Vec::new();
    for fmt in &m.formats {
        let Ok(pt) = fmt.as_str().parse::<u8>() else {
            continue;
        };
        if let Some(rtpmap) = m.rtpmaps.get(&pt) {
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
            if let Some((name, rate)) = static_pt_info(pt) {
                codecs.push(SdpCodecInfo {
                    payload_type: pt,
                    encoding_name: name.to_string(),
                    clock_rate: rate,
                });
            }
        }
    }
    codecs
}

/// The telephone-event payload type and clock rate of one section.
fn telephone_event_of(m: &MediaDescription) -> Option<(u8, u32)> {
    m.rtpmaps
        .iter()
        .find(|(_, r)| {
            r.encoding_name
                .as_str()
                .eq_ignore_ascii_case("telephone-event")
        })
        .map(|(pt, r)| (*pt, r.clock_rate))
}

/// Parse audio codecs from SDP with payload types and clock rates.
///
/// Reads the first audio section with `sip_sdp::SessionDescription::parse()`,
/// with a manual fallback scoped to that section for malformed SDP. Filters
/// out telephone-event entries.
pub fn parse_sdp_codecs(sdp_body: &str) -> Vec<SdpCodecInfo> {
    if let Ok(sdp) = SessionDescription::parse(sdp_body) {
        if let Some(audio) = first_section(&sdp, MediaType::Audio) {
            return codecs_of_section(audio);
        }
    }
    parse_sdp_codecs_manual(sdp_body)
}

/// Extract the telephone-event payload type from the first audio section.
/// Returns `None` if absent.
pub fn extract_telephone_event_pt(sdp_body: &str) -> Option<u8> {
    if let Ok(sdp) = SessionDescription::parse(sdp_body) {
        if let Some(audio) = first_section(&sdp, MediaType::Audio) {
            if let Some((pt, _)) = telephone_event_of(audio) {
                return Some(pt);
            }
        }
        // Parsed, but no telephone-event in the rtpmaps — fall through to
        // the manual scan (sip_sdp may not populate rtpmaps for high
        // dynamic PTs like 127 on some SDP layouts).
    }
    for line in first_section_lines(sdp_body, "audio") {
        if let Some(rtpmap_data) = line.strip_prefix("a=rtpmap:") {
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

/// Manual fallback parser for malformed SDP, scoped to the first audio
/// section.
fn parse_sdp_codecs_manual(sdp_body: &str) -> Vec<SdpCodecInfo> {
    let mut payload_types: Vec<String> = Vec::new();
    let mut rtpmap_info: HashMap<String, (String, u32)> = HashMap::new();

    for line in first_section_lines(sdp_body, "audio") {
        if line.starts_with("m=audio") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 3 {
                payload_types = parts[3..].iter().map(|s| s.to_string()).collect();
            }
        }
        if let Some(rtpmap_data) = line.strip_prefix("a=rtpmap:") {
            if let Some((pt, codec_info)) = rtpmap_data.split_once(' ') {
                let parts: Vec<&str> = codec_info.split('/').collect();
                let codec_name = parts[0];
                let clock_rate = parts
                    .get(1)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(8000);
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

/// The `a=crypto:` lines of one section.
fn crypto_of_section(m: &MediaDescription) -> Vec<SdesCryptoLine> {
    let mut lines = Vec::new();
    for attr in &m.attributes {
        if let Attribute::Value { name, value } = attr {
            if name.as_str() == "crypto" {
                if let Some(parsed) = parse_crypto_value(value.as_str()) {
                    lines.push(parsed);
                }
            }
        }
    }
    lines
}

/// Parse the `a=crypto:` lines of the first audio section (RFC 4568).
///
/// Uses the `sip_sdp` parser first, falling back to a manual scan of that
/// section for malformed SDP. Returns an empty vec if no crypto lines are
/// found (plain RTP).
pub fn parse_sdes_crypto(sdp_body: &str) -> Vec<SdesCryptoLine> {
    if let Ok(sdp) = SessionDescription::parse(sdp_body) {
        if let Some(audio) = first_section(&sdp, MediaType::Audio) {
            let lines = crypto_of_section(audio);
            if !lines.is_empty() {
                return lines;
            }
        }
    }
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

/// Manual fallback for SDES crypto parsing, scoped to the first audio
/// section.
fn parse_sdes_crypto_manual(sdp_body: &str) -> Vec<SdesCryptoLine> {
    first_section_lines(sdp_body, "audio")
        .into_iter()
        .filter_map(|line| line.strip_prefix("a=crypto:").and_then(parse_crypto_value))
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
            suite,
            expected,
            decoded.len()
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
    Ok(sip_sdp.to_string())
}

/// Convert forge-media SDP to SIP SDP format
pub fn convert_forge_to_sip_sdp(forge_sdp: &str) -> Result<String> {
    Ok(forge_sdp.to_string())
}

// ── Remote endpoints ────────────────────────────────────────────────────

/// The remote RTP endpoint of the first active section of a type: its
/// port with its own `c=` line, or the session-level one. `None` when the
/// section is absent or disabled (port 0).
fn remote_endpoint(sdp_body: &str, media: MediaType) -> Option<SocketAddr> {
    if let Ok(sdp) = SessionDescription::parse(sdp_body) {
        let m = sdp
            .media
            .iter()
            .find(|m| m.media_type == media && m.port != 0)?;
        let conn = m.connection.as_ref().or(sdp.connection.as_ref())?;
        let ip: IpAddr = conn.connection_address.as_str().parse().ok()?;
        return Some(SocketAddr::new(ip, m.port));
    }
    // Manual fallback: session-level c=, then the section's own.
    let name = match media {
        MediaType::Audio => "audio",
        MediaType::Video => "video",
        _ => return None,
    };
    let mut session_ip: Option<&str> = None;
    for line in sdp_body.lines() {
        let t = line.trim();
        if t.starts_with("m=") {
            break;
        }
        if let Some(rest) = t.strip_prefix("c=IN ") {
            session_ip = rest.split_whitespace().nth(1);
        }
    }
    let lines = first_section_lines(sdp_body, name);
    let port: u16 = lines.first()?.split_whitespace().nth(1)?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let ip = lines
        .iter()
        .find_map(|l| {
            l.strip_prefix("c=IN ")
                .and_then(|r| r.split_whitespace().nth(1))
        })
        .or(session_ip)?;
    let ip: IpAddr = ip.parse().ok()?;
    Some(SocketAddr::new(ip, port))
}

/// The remote audio RTP endpoint (first active audio section).
pub fn remote_audio_endpoint(sdp_body: &str) -> Option<SocketAddr> {
    remote_endpoint(sdp_body, MediaType::Audio)
}

/// The remote video RTP endpoint (first active video section).
pub fn remote_video_endpoint(sdp_body: &str) -> Option<SocketAddr> {
    remote_endpoint(sdp_body, MediaType::Video)
}

// ── Video offers ────────────────────────────────────────────────────────

/// One codec of an offered video section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoOfferCodec {
    pub payload_type: u8,
    pub codec: VideoCodec,
    /// The offered `a=fmtp` parameters for this payload type, if any.
    pub fmtp: Option<String>,
}

/// What a peer offered for video: the first active video section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoOffer {
    pub port: u16,
    pub protocol: String,
    /// Codecs FCP knows, in the offer's order.
    pub codecs: Vec<VideoOfferCodec>,
    pub remote: Option<SocketAddr>,
    /// Whether the section carries any `a=rtcp-fb` (PLI, FIR, NACK, …).
    pub feedback: bool,
    /// Whether the section is encrypted (SDES `a=crypto` present).
    pub crypto: bool,
}

/// Parse the first active video section of an offer, or `None` when the
/// offer has no video (or only a disabled section).
pub fn parse_video_offer(sdp_body: &str) -> Option<VideoOffer> {
    let sdp = SessionDescription::parse(sdp_body).ok()?;
    let m = forge_sdp::active_video(&sdp)?;
    let codecs = m
        .video_codecs()
        .into_iter()
        .map(|(ci, codec)| VideoOfferCodec {
            payload_type: ci.payload_type,
            codec,
            fmtp: m.fmtp_for(ci.payload_type).map(|f| f.params.to_string()),
        })
        .collect();
    Some(VideoOffer {
        port: m.port,
        protocol: m.protocol.to_string(),
        codecs,
        remote: remote_endpoint(sdp_body, MediaType::Video),
        feedback: !m.rtcp_fb_iter().is_empty(),
        crypto: !crypto_of_section(m).is_empty(),
    })
}

// ── Answers ─────────────────────────────────────────────────────────────

/// SRTP context for SDP rewriting.
///
/// When provided to SDP rewriting functions, causes `RTP/AVP` to be replaced
/// with `RTP/SAVP` on the `m=audio` line and an `a=crypto:` attribute to be
/// injected.  Existing `a=crypto:` lines are stripped first.
#[derive(Debug, Clone)]
pub struct SrtpSdpContext {
    /// Base64-encoded SRTP key material (e.g., `"inline:<base64>"`).
    pub local_key_b64: String,
    /// Crypto suite name (e.g., `"AES_CM_128_HMAC_SHA1_80"`).
    pub suite: String,
    /// Crypto tag (typically `1`).
    pub tag: u32,
}

impl SrtpSdpContext {
    fn crypto_value(&self) -> String {
        format!("{} {} {}", self.tag, self.suite, self.local_key_b64)
    }
}

/// How to answer an offered video section.
#[derive(Debug, Clone)]
pub struct VideoAnswer<'a> {
    /// Our RTP port for video.
    pub rtp_port: u16,
    /// Codec preference; the first one the offer also carries is chosen.
    pub codecs: &'a [VideoCodec],
    /// SRTP for the video section (its own key); `None` answers plain RTP.
    pub srtp: Option<&'a SrtpSdpContext>,
}

/// Everything an answer needs besides the offer.
#[derive(Debug, Clone)]
pub struct AnswerOptions<'a> {
    pub server_ip: &'a str,
    pub audio_rtp_port: u16,
    pub audio_srtp: Option<&'a SrtpSdpContext>,
    /// `None` rejects every offered video section (port 0).
    pub video: Option<VideoAnswer<'a>>,
}

/// What the video section of an answer agreed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedVideo {
    pub codec: VideoCodec,
    pub payload_type: u8,
    /// The offered `a=fmtp` for the chosen payload type (echoed in the answer).
    pub fmtp: Option<String>,
    pub remote: Option<SocketAddr>,
    pub pli: bool,
    pub fir: bool,
    pub nack: bool,
    /// The peer's first crypto line for the video section, if it offered one.
    pub remote_crypto: Option<String>,
}

/// An answer and what it agreed.
#[derive(Debug, Clone)]
pub struct Answer {
    pub sdp: String,
    /// `Some` when a video section was accepted.
    pub video: Option<NegotiatedVideo>,
}

/// The protocol to answer with: secure when we have SRTP, feedback (`…F`)
/// when the offer had it.
fn answer_protocol(offered: &Protocol, srtp: bool) -> Protocol {
    let feedback = matches!(
        offered,
        Protocol::RtpAvpf
            | Protocol::RtpSavpf
            | Protocol::UdpTlsRtpSavpf
            | Protocol::TcpTlsRtpSavpf
    );
    match (srtp, feedback) {
        (true, true) => Protocol::RtpSavpf,
        (true, false) => Protocol::RtpSavp,
        (false, true) => Protocol::RtpAvpf,
        (false, false) => Protocol::RtpAvp,
    }
}

fn sdp_err(e: impl std::fmt::Display) -> Error {
    Error::Sdp(e.to_string())
}

/// The audio answer FCP has always given: the offer's first codec, its
/// telephone-event type, our port, `a=crypto` when we have SRTP, and the
/// direction that answers the offer's.
fn audio_answer_section(
    m: &MediaDescription,
    opts: &AnswerOptions<'_>,
) -> Result<MediaDescription> {
    let codecs = codecs_of_section(m);
    let mut a = MediaDescription::audio(opts.audio_rtp_port);
    a.protocol = answer_protocol(&m.protocol, opts.audio_srtp.is_some());
    if let Some(ctx) = opts.audio_srtp {
        a = a
            .add_attribute("crypto", &ctx.crypto_value())
            .map_err(sdp_err)?;
    }
    if let Some(c) = codecs.first() {
        let params = m
            .rtpmaps
            .get(&c.payload_type)
            .and_then(|r| r.encoding_params.as_ref())
            .map(|p| p.to_string());
        a = a.add_format(c.payload_type).map_err(sdp_err)?;
        a = a
            .add_rtpmap(
                c.payload_type,
                &c.encoding_name,
                c.clock_rate,
                params.as_deref(),
            )
            .map_err(sdp_err)?;
    }
    if let Some((te, rate)) = telephone_event_of(m) {
        a = a.add_format(te).map_err(sdp_err)?;
        a = a
            .add_rtpmap(te, "telephone-event", rate, None)
            .map_err(sdp_err)?;
        a = a
            .add_attribute("fmtp", &format!("{te} 0-16"))
            .map_err(sdp_err)?;
    }
    a = a
        .add_property(answer_direction(direction_of(m), "sendrecv"))
        .map_err(sdp_err)?;
    Ok(a)
}

/// The first codec of `preference` the offer carries. Unlike
/// [`choose_video_codec`], nothing outside `preference` is ever chosen: a
/// room that allows only AV1 must reject a VP8 offer.
fn choose_allowed_video_codec(
    m: &MediaDescription,
    preference: &[VideoCodec],
) -> Option<(forge_sdp::CodecInfo, VideoCodec)> {
    let offered = m.video_codecs();
    preference
        .iter()
        .find_map(|want| offered.iter().find(|(_, have)| have == want).cloned())
}

/// Answer an offered video section, or reject it when no allowed codec is
/// shared.
fn video_answer_section(
    m: &MediaDescription,
    v: &VideoAnswer<'_>,
    remote: Option<SocketAddr>,
) -> (MediaDescription, Option<NegotiatedVideo>) {
    let Some((ci, codec)) = choose_allowed_video_codec(m, v.codecs) else {
        return (reject_section(m), None);
    };
    let mut sec = answer_video(m, &ci, v.rtp_port, "sendrecv");
    sec.protocol = answer_protocol(&m.protocol, v.srtp.is_some());
    if let Some(ctx) = v.srtp {
        sec.attributes
            .retain(|a| !matches!(a, Attribute::Value { name, .. } if name.as_str() == "crypto"));
        sec.attributes.insert(
            0,
            Attribute::Value {
                name: "crypto".into(),
                value: ctx.crypto_value().into(),
            },
        );
    }
    let pt = ci.payload_type;
    let negotiated = NegotiatedVideo {
        codec,
        payload_type: pt,
        fmtp: m.fmtp_for(pt).map(|f| f.params.to_string()),
        remote,
        pli: m.supports_pli(pt),
        fir: m.supports_fir(pt),
        nack: m.rtcp_fb_for(pt).iter().any(|f| f.is_nack()),
        remote_crypto: crypto_of_section(m)
            .first()
            .map(|c| format!("{} {} {}", c.tag, c.suite, c.key_params)),
    };
    (sec, Some(negotiated))
}

/// One answer section per offered section, in order (RFC 3264 §6): the
/// first active audio section through `audio`, the first active video
/// section through `video` (rejected when `None`), everything else
/// rejected with port 0.
pub fn answer_sections(
    offer: &SessionDescription,
    offer_body: &str,
    mut audio: impl FnMut(&MediaDescription) -> Result<MediaDescription>,
    video: Option<&VideoAnswer<'_>>,
) -> Result<(Vec<MediaDescription>, Option<NegotiatedVideo>)> {
    let mut sections = Vec::with_capacity(offer.media.len());
    let mut audio_done = false;
    let mut negotiated = None;
    for m in &offer.media {
        let section = match m.media_type {
            MediaType::Audio if !audio_done && m.port != 0 => {
                audio_done = true;
                audio(m)?
            }
            MediaType::Video if negotiated.is_none() && m.port != 0 => match video {
                Some(v) => {
                    let remote = remote_endpoint(offer_body, MediaType::Video);
                    let (sec, neg) = video_answer_section(m, v, remote);
                    negotiated = neg;
                    sec
                }
                None => reject_section(m),
            },
            _ => reject_section(m),
        };
        sections.push(section);
    }
    Ok((sections, negotiated))
}

/// Serialise answer sections under FCP's session-level lines.
pub fn serialize_answer(
    server_ip: &str,
    origin_id: &str,
    session_name: &str,
    sections: Vec<MediaDescription>,
) -> Result<String> {
    let mut b = SessionDescription::builder()
        .origin("fcp", origin_id, server_ip)
        .map_err(sdp_err)?
        .session_name(session_name)
        .map_err(sdp_err)?
        .connection(server_ip)
        .map_err(sdp_err)?
        .time(0, 0);
    for s in sections {
        b = b.media(s).map_err(sdp_err)?;
    }
    Ok(b.build().serialize())
}

/// Construct an SDP answer to `offer`: the offer's preferred audio codec on
/// our audio port, the video section answered per `opts.video` or rejected
/// with port 0, every other section rejected. Fails only when the offer
/// does not parse; callers that must always answer use
/// [`construct_answer_for_a_leg`], which falls back to an audio-only answer.
pub fn construct_answer(offer: &str, opts: &AnswerOptions<'_>) -> Result<Answer> {
    let parsed = SessionDescription::parse(offer).map_err(sdp_err)?;
    if parsed.media.is_empty() {
        return Err(Error::Sdp("offer has no media sections".into()));
    }
    let (sections, video) = answer_sections(
        &parsed,
        offer,
        |m| audio_answer_section(m, opts),
        opts.video.as_ref(),
    )?;
    let sdp = serialize_answer(opts.server_ip, "0", "FCP Media Server", sections)?;
    Ok(Answer { sdp, video })
}

/// Construct an SDP answer for A-leg using A-leg's preferred codec.
///
/// When B-leg answers with a different codec than A-leg's preference,
/// we construct a new SDP answer for A-leg using A-leg's preferred codec.
/// The transcoder handles the conversion between A-leg and B-leg codecs.
/// Any video or other section in the offer is rejected properly (port 0,
/// the offer's formats), so the answer has one section per offered section.
///
/// # Arguments
/// * `a_leg_offer` - A-leg's original SDP offer
/// * `server_ip` - Server's IP address for media anchoring
/// * `rtp_port` - Server's RTP port for media anchoring
/// * `srtp` - Optional SRTP context; when `Some`, uses `RTP/SAVP` and
///   appends an `a=crypto:` attribute.
pub fn construct_answer_for_a_leg(
    a_leg_offer: &str,
    server_ip: &str,
    rtp_port: u16,
    srtp: Option<&SrtpSdpContext>,
) -> String {
    let opts = AnswerOptions {
        server_ip,
        audio_rtp_port: rtp_port,
        audio_srtp: srtp,
        video: None,
    };
    match construct_answer(a_leg_offer, &opts) {
        Ok(a) => a.sdp,
        // The offer did not parse: answer audio from what can be scraped.
        Err(_) => legacy_audio_answer(a_leg_offer, server_ip, rtp_port, srtp),
    }
}

/// The pre-section answer: an audio-only answer built from string scans,
/// kept for offers the parser rejects.
fn legacy_audio_answer(
    a_leg_offer: &str,
    server_ip: &str,
    rtp_port: u16,
    srtp: Option<&SrtpSdpContext>,
) -> String {
    let codecs = parse_sdp_codecs(a_leg_offer);
    let te_pt = extract_telephone_event_pt(a_leg_offer);
    let preferred = codecs.first();

    let mut sdp = String::new();
    sdp.push_str("v=0\r\n");
    sdp.push_str("o=fcp 0 0 IN IP4 ");
    sdp.push_str(server_ip);
    sdp.push_str("\r\n");
    sdp.push_str("s=FCP Media Server\r\n");
    sdp.push_str("c=IN IP4 ");
    sdp.push_str(server_ip);
    sdp.push_str("\r\n");
    sdp.push_str("t=0 0\r\n");

    let protocol = if srtp.is_some() {
        "RTP/SAVP"
    } else {
        "RTP/AVP"
    };
    sdp.push_str(&format!("m=audio {} {}", rtp_port, protocol));
    if let Some(codec) = preferred {
        sdp.push_str(&format!(" {}", codec.payload_type));
    }
    if let Some(te) = te_pt {
        sdp.push_str(&format!(" {}", te));
    }
    sdp.push_str("\r\n");
    if let Some(ctx) = srtp {
        sdp.push_str(&format!("a=crypto:{}\r\n", ctx.crypto_value()));
    }
    if let Some(codec) = preferred {
        sdp.push_str(&format!(
            "a=rtpmap:{} {}/{}\r\n",
            codec.payload_type, codec.encoding_name, codec.clock_rate
        ));
    }
    if let Some(te) = te_pt {
        sdp.push_str(&format!("a=rtpmap:{} telephone-event/8000\r\n", te));
        sdp.push_str(&format!("a=fmtp:{} 0-16\r\n", te));
    }
    sdp.push_str("a=sendrecv\r\n");
    sdp
}

// ── Anchoring ───────────────────────────────────────────────────────────

/// The server ports an anchored SDP points at, per section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorPorts {
    /// Our audio RTP port.
    pub audio: u16,
    /// Our video RTP port; `None` disables the video section (port 0),
    /// since nothing would relay it.
    pub video: Option<u16>,
}

/// Swap a plain RTP profile for its secure counterpart (`RTP/AVP` →
/// `RTP/SAVP`, `RTP/AVPF` → `RTP/SAVPF`).
fn secure_protocol(protocol: &str) -> String {
    match protocol {
        "RTP/AVP" => "RTP/SAVP".to_string(),
        "RTP/AVPF" => "RTP/SAVPF".to_string(),
        other => other.to_string(),
    }
}

/// Rewrite an SDP so every section points at the server: every `c=` line
/// becomes `server_ip`; the first audio section takes `ports.audio` (and
/// `RTP/SAVP` plus our `a=crypto` when `srtp` is given, existing crypto
/// lines stripped); a video section takes `ports.video` or is disabled
/// with port 0; every other section is disabled. A disabled section keeps
/// its formats and attributes, as RFC 3264 requires.
pub fn rewrite_for_anchoring(
    original_sdp: &str,
    server_ip: &str,
    ports: AnchorPorts,
    srtp: Option<&SrtpSdpContext>,
) -> String {
    let mut result = String::new();
    let mut audio_seen = false;
    let mut in_anchored_audio = false;
    let mut crypto_injected = false;

    for line in original_sdp.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("m=") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let kind = parts[0];
                let (port, protocol) = match kind {
                    "audio" if !audio_seen => {
                        audio_seen = true;
                        in_anchored_audio = true;
                        let protocol = if srtp.is_some() {
                            secure_protocol(parts[2])
                        } else {
                            parts[2].to_string()
                        };
                        (ports.audio.to_string(), protocol)
                    }
                    "video" if ports.video.is_some() => {
                        in_anchored_audio = false;
                        (ports.video.unwrap_or(0).to_string(), parts[2].to_string())
                    }
                    _ => {
                        in_anchored_audio = false;
                        ("0".to_string(), parts[2].to_string())
                    }
                };
                result.push_str(&format!("m={kind} {port} {protocol}"));
                for part in &parts[3..] {
                    result.push(' ');
                    result.push_str(part);
                }
                result.push_str("\r\n");
                if in_anchored_audio {
                    if let Some(ctx) = srtp {
                        if !crypto_injected {
                            result.push_str(&format!("a=crypto:{}\r\n", ctx.crypto_value()));
                            crypto_injected = true;
                        }
                    }
                }
                continue;
            }
        }

        // Our key replaces the peer's in the audio section.
        if srtp.is_some() && in_anchored_audio && trimmed.starts_with("a=crypto:") {
            continue;
        }

        // Every connection line (session or media level) points at us.
        if trimmed.starts_with("c=IN IP") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 3 {
                result.push_str(&format!("c=IN {} {}\r\n", parts[1], server_ip));
                continue;
            }
        }

        result.push_str(line);
        result.push_str("\r\n");
    }

    result
}

/// Rewrite SDP to point to the server's RTP endpoint for audio.
///
/// Redirects the audio section through the server and disables every
/// other section (port 0): see [`rewrite_for_anchoring`] for a video port.
///
/// # Arguments
/// * `original_sdp` - The original SDP body
/// * `server_ip` - The server's IP address for RTP
/// * `rtp_port` - The audio RTP port allocated by forge-media
/// * `srtp` - Optional SRTP context; when `Some`, replaces `RTP/AVP` with
///   `RTP/SAVP`, strips existing `a=crypto:` lines, and injects a new one.
pub fn rewrite_sdp_for_anchoring(
    original_sdp: &str,
    server_ip: &str,
    rtp_port: u16,
    srtp: Option<&SrtpSdpContext>,
) -> String {
    rewrite_for_anchoring(
        original_sdp,
        server_ip,
        AnchorPorts {
            audio: rtp_port,
            video: None,
        },
        srtp,
    )
}

// ── Codec filtering ─────────────────────────────────────────────────────

/// Filter the audio section of an SDP to the given codecs.
///
/// Used by the B2BUA to constrain a B-leg offer to codecs the A-leg
/// supports, ensuring codec alignment or controlled transcoding. Only the
/// first audio section is filtered: its format list and its `a=rtpmap` /
/// `a=fmtp` lines. telephone-event is always kept. Video and other
/// sections pass through untouched.
pub fn filter_sdp_codecs(original_sdp: &str, allowed_codecs: &[&str]) -> String {
    let codecs = parse_sdp_codecs(original_sdp);

    let mut allowed_pts: Vec<u8> = codecs
        .iter()
        .filter(|c| {
            allowed_codecs
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&c.encoding_name))
        })
        .map(|c| c.payload_type)
        .collect();

    if let Some(te_pt) = extract_telephone_event_pt(original_sdp) {
        if !allowed_pts.contains(&te_pt) {
            allowed_pts.push(te_pt);
        }
    }

    let mut result = String::new();
    let mut audio_seen = false;
    let mut in_audio = false;
    for line in original_sdp.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("m=") {
            in_audio = false;
            if trimmed.starts_with("m=audio ") && !audio_seen {
                audio_seen = true;
                in_audio = true;
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
        }

        if in_audio && (trimmed.starts_with("a=rtpmap:") || trimmed.starts_with("a=fmtp:")) {
            let attr_data = match trimmed.strip_prefix("a=rtpmap:") {
                Some(rest) => rest,
                None => &trimmed["a=fmtp:".len()..],
            };
            if let Some(pt_str) = attr_data.split_whitespace().next() {
                if let Ok(pt) = pt_str.parse::<u8>() {
                    if !allowed_pts.contains(&pt) {
                        continue;
                    }
                }
            }
        }

        result.push_str(line);
        result.push_str("\r\n");
    }

    result
}

/// Extract audio codec names from the first audio section of an SDP body.
///
/// ```text
/// m=audio 10000 RTP/AVP 0 8 101
/// a=rtpmap:0 PCMU/8000
/// a=rtpmap:8 PCMA/8000
/// a=rtpmap:101 telephone-event/8000
/// ```
///
/// Returns: `["PCMU", "PCMA"]` (telephone-event is skipped).
pub fn extract_codecs_from_sdp(sdp_body: &str) -> Vec<String> {
    let mut codecs = Vec::new();
    let mut payload_types: Vec<String> = Vec::new();
    let mut rtpmap: HashMap<String, String> = HashMap::new();

    for line in first_section_lines(sdp_body, "audio") {
        if line.starts_with("m=audio") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 3 {
                payload_types = parts[3..].iter().map(|s| s.to_string()).collect();
            }
        }
        if let Some(rtpmap_data) = line.strip_prefix("a=rtpmap:") {
            if let Some((pt, codec_info)) = rtpmap_data.split_once(' ') {
                let codec_name = codec_info.split('/').next().unwrap_or(codec_info);
                rtpmap.insert(pt.to_string(), codec_name.to_string());
            }
        }
    }

    for pt in &payload_types {
        if let Some(codec) = rtpmap.get(pt) {
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

        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000, None);

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

        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000, None);

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

        let rewritten = rewrite_sdp_for_anchoring(original, "2001:db8::5", 20000, None);

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

    // ── SRTP SDP rewriting tests ─────────────────────────────────────────

    #[test]
    fn test_rewrite_sdp_strips_and_injects_crypto() {
        let original = "v=0\r\n\
                         o=- 123 1 IN IP4 192.168.1.100\r\n\
                         s=Test\r\n\
                         c=IN IP4 192.168.1.100\r\n\
                         t=0 0\r\n\
                         m=audio 10000 RTP/AVP 0 8\r\n\
                         a=rtpmap:0 PCMU/8000\r\n\
                         a=rtpmap:8 PCMA/8000\r\n\
                         a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:oldkey123\r\n";

        let ctx = SrtpSdpContext {
            local_key_b64: "inline:newkey456".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000, Some(&ctx));

        // Protocol should be RTP/SAVP
        assert!(rewritten.contains("m=audio 20000 RTP/SAVP 0 8"));
        // Old crypto line stripped
        assert!(!rewritten.contains("inline:oldkey123"));
        // New crypto line injected
        assert!(rewritten.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:newkey456"));
        // Connection address updated
        assert!(rewritten.contains("c=IN IP4 10.0.0.5"));
        // Codecs preserved
        assert!(rewritten.contains("a=rtpmap:0 PCMU/8000"));
        assert!(rewritten.contains("a=rtpmap:8 PCMA/8000"));
    }

    #[test]
    fn test_rewrite_sdp_none_context_passthrough() {
        let original = "v=0\r\n\
                         o=- 123 1 IN IP4 192.168.1.100\r\n\
                         s=Test\r\n\
                         c=IN IP4 192.168.1.100\r\n\
                         t=0 0\r\n\
                         m=audio 10000 RTP/AVP 0\r\n\
                         a=rtpmap:0 PCMU/8000\r\n\
                         a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:existingkey\r\n";

        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000, None);

        // Protocol stays RTP/AVP (no SRTP)
        assert!(rewritten.contains("m=audio 20000 RTP/AVP 0"));
        // Existing crypto line preserved (no stripping when None)
        assert!(rewritten.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:existingkey"));
        // No new crypto lines injected
        assert_eq!(
            rewritten.matches("a=crypto:").count(),
            1,
            "should have exactly one crypto line (the original)"
        );
    }

    #[test]
    fn test_construct_answer_with_srtp() {
        let offer = "v=0\r\n\
                      o=- 123 1 IN IP4 192.168.1.100\r\n\
                      s=Test\r\n\
                      c=IN IP4 192.168.1.100\r\n\
                      t=0 0\r\n\
                      m=audio 10000 RTP/AVP 0 101\r\n\
                      a=rtpmap:0 PCMU/8000\r\n\
                      a=rtpmap:101 telephone-event/8000\r\n\
                      a=fmtp:101 0-16\r\n";

        let ctx = SrtpSdpContext {
            local_key_b64: "inline:answersrtpkey".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let answer = construct_answer_for_a_leg(offer, "10.0.0.5", 20000, Some(&ctx));

        // Protocol should be RTP/SAVP
        assert!(answer.contains("m=audio 20000 RTP/SAVP"));
        // Crypto line present
        assert!(answer.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:answersrtpkey"));
        // Codec and TE preserved
        assert!(answer.contains("a=rtpmap:0 PCMU/8000"));
        assert!(answer.contains("a=rtpmap:101 telephone-event/8000"));
    }

    #[test]
    fn test_construct_answer_without_srtp() {
        let offer = "v=0\r\n\
                      o=- 123 1 IN IP4 192.168.1.100\r\n\
                      s=Test\r\n\
                      c=IN IP4 192.168.1.100\r\n\
                      t=0 0\r\n\
                      m=audio 10000 RTP/AVP 0 101\r\n\
                      a=rtpmap:0 PCMU/8000\r\n\
                      a=rtpmap:101 telephone-event/8000\r\n\
                      a=fmtp:101 0-16\r\n";

        let answer = construct_answer_for_a_leg(offer, "10.0.0.5", 20000, None);

        // Protocol should stay RTP/AVP
        assert!(answer.contains("m=audio 20000 RTP/AVP"));
        // No crypto line
        assert!(!answer.contains("a=crypto:"));
        // Codec preserved
        assert!(answer.contains("a=rtpmap:0 PCMU/8000"));
    }
}

#[test]
fn test_polycom_filter_preserves_telephone_event() {
    let polycom_sdp = "v=0\r\no=- 1774033578 1774033578 IN IP4 192.168.1.12\r\ns=Polycom IP Phone\r\nc=IN IP4 192.168.1.12\r\nt=0 0\r\na=sendrecv\r\nm=audio 2236 RTP/AVP 0 9 102 8 18 127\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:9 G722/8000\r\na=rtpmap:102 G7221/16000\r\na=fmtp:102 bitrate=32000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:18 G729/8000\r\na=fmtp:18 annexb=no\r\na=rtpmap:127 telephone-event/8000\r\n";

    // Step 1: Filter as call manager does
    let filtered = filter_sdp_codecs(polycom_sdp, &["PCMU", "PCMA"]);
    eprintln!("Filtered SDP:\n{}", filtered);

    // Verify telephone-event survived filtering
    let te_after_filter = extract_telephone_event_pt(&filtered);
    assert_eq!(
        te_after_filter,
        Some(127),
        "telephone-event lost during filtering"
    );

    // Step 2: Construct answer as conference server does
    let answer = construct_answer_for_a_leg(&filtered, "192.168.1.225", 26708, None);
    eprintln!("Answer SDP:\n{}", answer);

    // Verify answer includes telephone-event
    assert!(answer.contains("127"), "answer m= line missing PT 127");
    assert!(
        answer.contains("telephone-event"),
        "answer missing telephone-event rtpmap"
    );
}

#[cfg(test)]
mod section_tests {
    use super::*;

    const AV_OFFER: &str = "v=0\r\n\
        o=- 1 1 IN IP4 192.168.1.50\r\n\
        s=phone\r\n\
        c=IN IP4 192.168.1.50\r\n\
        t=0 0\r\n\
        m=audio 4000 RTP/AVP 0 8 101\r\n\
        a=rtpmap:0 PCMU/8000\r\n\
        a=rtpmap:8 PCMA/8000\r\n\
        a=rtpmap:101 telephone-event/8000\r\n\
        a=fmtp:101 0-15\r\n\
        a=sendrecv\r\n\
        m=video 4002 RTP/AVPF 96 97\r\n\
        c=IN IP4 192.168.1.51\r\n\
        a=rtpmap:96 VP8/90000\r\n\
        a=rtpmap:97 H264/90000\r\n\
        a=fmtp:97 profile-level-id=42e01f;packetization-mode=1\r\n\
        a=rtcp-fb:* nack pli\r\n\
        a=rtcp-fb:* ccm fir\r\n\
        a=sendrecv\r\n";

    fn sections(sdp: &str) -> Vec<String> {
        sdp.lines()
            .filter(|l| l.starts_with("m="))
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn video_is_rejected_properly_when_not_allowed() {
        let answer = construct_answer_for_a_leg(AV_OFFER, "10.0.0.5", 20000, None);
        let m = sections(&answer);
        assert_eq!(m.len(), 2, "{answer}");
        assert_eq!(m[0], "m=audio 20000 RTP/AVP 0 101");
        // Port 0, the offer's protocol and formats (RFC 3264 §6).
        assert_eq!(m[1], "m=video 0 RTP/AVPF 96 97");
        assert!(answer.contains("a=rtpmap:0 PCMU/8000"));
        assert!(answer.contains("a=rtpmap:101 telephone-event/8000"));
        assert!(answer.contains("a=fmtp:101 0-16"));
        assert!(answer.contains("c=IN IP4 10.0.0.5"));
        // The rejected section carries no feedback or crypto of ours.
        let video_part = &answer[answer.find("m=video").unwrap()..];
        assert!(video_part.contains("a=inactive"));
        assert!(!video_part.contains("rtcp-fb"));
    }

    #[test]
    fn video_is_answered_when_allowed() {
        let codecs = [VideoCodec::H264, VideoCodec::VP8];
        let opts = AnswerOptions {
            server_ip: "10.0.0.5",
            audio_rtp_port: 20000,
            audio_srtp: None,
            video: Some(VideoAnswer {
                rtp_port: 20002,
                codecs: &codecs,
                srtp: None,
            }),
        };
        let answer = construct_answer(AV_OFFER, &opts).unwrap();
        let m = sections(&answer.sdp);
        assert_eq!(m[0], "m=audio 20000 RTP/AVP 0 101");
        // Our preference (H.264) wins over the offer's order (VP8 first);
        // the offer's payload type and feedback profile are kept.
        assert_eq!(m[1], "m=video 20002 RTP/AVPF 97");
        let video_part = &answer.sdp[answer.sdp.find("m=video").unwrap()..];
        assert!(video_part.contains("a=rtpmap:97 H264/90000"));
        assert!(video_part.contains("a=fmtp:97 profile-level-id=42e01f;packetization-mode=1"));
        assert!(video_part.contains("a=rtcp-fb:97 nack pli"));
        assert!(video_part.contains("a=rtcp-fb:97 ccm fir"));
        assert!(video_part.contains("a=sendrecv"));
        let v = answer.video.expect("video negotiated");
        assert_eq!(v.codec, VideoCodec::H264);
        assert_eq!(v.payload_type, 97);
        assert_eq!(
            v.fmtp.as_deref(),
            Some("profile-level-id=42e01f;packetization-mode=1")
        );
        // The offer carried `nack pli` and `ccm fir`, not plain `nack`.
        assert!(v.pli && v.fir && !v.nack);
        // The video section's own c= line wins for its endpoint.
        assert_eq!(v.remote, Some("192.168.1.51:4002".parse().unwrap()));
        assert!(v.remote_crypto.is_none());
    }

    #[test]
    fn video_without_a_shared_codec_is_rejected_even_when_allowed() {
        let codecs = [VideoCodec::AV1];
        let opts = AnswerOptions {
            server_ip: "10.0.0.5",
            audio_rtp_port: 20000,
            audio_srtp: None,
            video: Some(VideoAnswer {
                rtp_port: 20002,
                codecs: &codecs,
                srtp: None,
            }),
        };
        let answer = construct_answer(AV_OFFER, &opts).unwrap();
        assert_eq!(sections(&answer.sdp)[1], "m=video 0 RTP/AVPF 96 97");
        assert!(answer.video.is_none());
    }

    #[test]
    fn srtp_applies_per_section_with_its_own_key() {
        let audio_ctx = SrtpSdpContext {
            local_key_b64: "inline:audiokey".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let video_ctx = SrtpSdpContext {
            local_key_b64: "inline:videokey".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let codecs = [VideoCodec::VP8];
        let opts = AnswerOptions {
            server_ip: "10.0.0.5",
            audio_rtp_port: 20000,
            audio_srtp: Some(&audio_ctx),
            video: Some(VideoAnswer {
                rtp_port: 20002,
                codecs: &codecs,
                srtp: Some(&video_ctx),
            }),
        };
        let answer = construct_answer(AV_OFFER, &opts).unwrap().sdp;
        let m = sections(&answer);
        assert_eq!(m[0], "m=audio 20000 RTP/SAVP 0 101");
        assert_eq!(m[1], "m=video 20002 RTP/SAVPF 96");
        let split = answer.find("m=video").unwrap();
        assert!(answer[..split].contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:audiokey"));
        assert!(answer[split..].contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:videokey"));
        assert!(!answer[split..].contains("audiokey"));
    }

    #[test]
    fn answer_direction_follows_the_offer() {
        let hold = AV_OFFER.replacen("a=sendrecv", "a=sendonly", 1);
        let answer = construct_answer_for_a_leg(&hold, "10.0.0.5", 20000, None);
        let audio_part = &answer[answer.find("m=audio").unwrap()..answer.find("m=video").unwrap()];
        assert!(audio_part.contains("a=recvonly"), "{answer}");
        assert!(!audio_part.contains("a=sendrecv"));
    }

    #[test]
    fn other_media_and_extra_audio_sections_are_rejected() {
        let offer = "v=0\r\no=- 1 1 IN IP4 10.1.1.1\r\ns=-\r\nc=IN IP4 10.1.1.1\r\nt=0 0\r\n\
                     m=application 5000 UDP/BFCP *\r\n\
                     m=audio 4000 RTP/AVP 8\r\n\
                     m=audio 4004 RTP/AVP 0\r\n";
        let answer = construct_answer_for_a_leg(offer, "10.0.0.5", 20000, None);
        let m = sections(&answer);
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], "m=application 0 UDP/BFCP *");
        assert_eq!(m[1], "m=audio 20000 RTP/AVP 8");
        assert_eq!(m[2], "m=audio 0 RTP/AVP 0");
    }

    #[test]
    fn unparseable_offer_still_gets_an_audio_answer() {
        let junk = "m=audio 4000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let answer = construct_answer_for_a_leg(junk, "10.0.0.5", 20000, None);
        assert!(answer.starts_with("v=0\r\n"));
        assert!(answer.contains("m=audio 20000 RTP/AVP 0\r\n"));
        assert!(answer.contains("a=rtpmap:0 PCMU/8000"));
    }

    #[test]
    fn anchoring_disables_video_and_repoints_every_connection_line() {
        let rewritten = rewrite_sdp_for_anchoring(AV_OFFER, "10.0.0.5", 20000, None);
        let m = sections(&rewritten);
        assert_eq!(m[0], "m=audio 20000 RTP/AVP 0 8 101");
        assert_eq!(m[1], "m=video 0 RTP/AVPF 96 97");
        // Both connection lines (session and the video section's own) now
        // point at us; the origin line is the peer's and stays.
        assert!(!rewritten.contains("c=IN IP4 192.168.1.5"), "{rewritten}");
        assert_eq!(rewritten.matches("c=IN IP4 10.0.0.5").count(), 2);
        // Formats and attributes of the disabled section are kept.
        assert!(rewritten.contains("a=rtpmap:96 VP8/90000"));
    }

    #[test]
    fn anchoring_with_a_video_port_keeps_video_live() {
        let rewritten = rewrite_for_anchoring(
            AV_OFFER,
            "10.0.0.5",
            AnchorPorts {
                audio: 20000,
                video: Some(20002),
            },
            None,
        );
        let m = sections(&rewritten);
        assert_eq!(m[1], "m=video 20002 RTP/AVPF 96 97");
    }

    #[test]
    fn anchoring_with_srtp_swaps_the_feedback_profile_too() {
        let offer = AV_OFFER.replace("m=audio 4000 RTP/AVP", "m=audio 4000 RTP/AVPF");
        let ctx = SrtpSdpContext {
            local_key_b64: "inline:k".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let rewritten = rewrite_sdp_for_anchoring(&offer, "10.0.0.5", 20000, Some(&ctx));
        assert_eq!(sections(&rewritten)[0], "m=audio 20000 RTP/SAVPF 0 8 101");
        assert_eq!(rewritten.matches("a=crypto:").count(), 1);
    }

    #[test]
    fn codec_filtering_leaves_the_video_section_alone() {
        let filtered = filter_sdp_codecs(AV_OFFER, &["PCMA"]);
        let m = sections(&filtered);
        assert_eq!(m[0], "m=audio 4000 RTP/AVP 8 101");
        assert_eq!(m[1], "m=video 4002 RTP/AVPF 96 97");
        assert!(!filtered.contains("a=rtpmap:0 PCMU/8000"));
        assert!(filtered.contains("a=rtpmap:96 VP8/90000"));
        assert!(filtered.contains("a=rtpmap:97 H264/90000"));
        assert!(filtered.contains("a=fmtp:97 profile-level-id=42e01f;packetization-mode=1"));
    }

    #[test]
    fn remote_endpoints_are_per_section() {
        assert_eq!(
            remote_audio_endpoint(AV_OFFER),
            Some("192.168.1.50:4000".parse().unwrap())
        );
        assert_eq!(
            remote_video_endpoint(AV_OFFER),
            Some("192.168.1.51:4002".parse().unwrap())
        );
        let disabled = AV_OFFER.replace("m=video 4002", "m=video 0");
        assert_eq!(remote_video_endpoint(&disabled), None);
        // Manual fallback on a body the parser rejects: same answers.
        let junk = "c=IN IP4 192.168.1.50\r\nm=audio 4000 RTP/AVP 0\r\nm=video 4002 RTP/AVP 96\r\nc=IN IP4 192.168.1.51\r\n";
        assert_eq!(
            remote_audio_endpoint(junk),
            Some("192.168.1.50:4000".parse().unwrap())
        );
        assert_eq!(
            remote_video_endpoint(junk),
            Some("192.168.1.51:4002".parse().unwrap())
        );
    }

    #[test]
    fn video_offers_are_summarised() {
        let v = parse_video_offer(AV_OFFER).expect("video offered");
        assert_eq!(v.port, 4002);
        assert_eq!(v.protocol, "RTP/AVPF");
        assert_eq!(
            v.codecs.iter().map(|c| c.codec).collect::<Vec<_>>(),
            vec![VideoCodec::VP8, VideoCodec::H264]
        );
        assert_eq!(
            v.codecs[1].fmtp.as_deref(),
            Some("profile-level-id=42e01f;packetization-mode=1")
        );
        assert!(v.feedback && !v.crypto);
        assert_eq!(v.remote, Some("192.168.1.51:4002".parse().unwrap()));
        let audio_only = &AV_OFFER[..AV_OFFER.find("m=video").unwrap()];
        assert!(parse_video_offer(audio_only).is_none());
    }

    #[test]
    fn manual_fallbacks_are_scoped_to_the_audio_section() {
        // Missing v= line: the parser rejects it, the scans must not read
        // the video rtpmaps or the video crypto line as audio.
        let junk = "m=audio 4000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\n\
                    a=rtpmap:101 telephone-event/8000\r\n\
                    m=video 4002 RTP/SAVP 96\r\na=rtpmap:96 VP8/90000\r\n\
                    a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:videokey\r\n";
        let codecs = parse_sdp_codecs(junk);
        assert_eq!(codecs.len(), 1);
        assert_eq!(codecs[0].encoding_name, "PCMU");
        assert_eq!(extract_telephone_event_pt(junk), Some(101));
        assert!(parse_sdes_crypto(junk).is_empty());
        assert_eq!(extract_codecs_from_sdp(junk), vec!["PCMU".to_string()]);
    }

    #[test]
    fn video_codec_conversions_round_trip() {
        for c in VideoCodec::ALL {
            assert_eq!(video_codec_to_forge(video_codec_from_forge(c)), c);
        }
        assert_eq!(
            video_codec_to_forge(fcp_core::VideoCodec::Av1),
            VideoCodec::AV1
        );
    }
}
