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

/// Which of a participant's video streams a section carries (RFC 4796):
/// the camera, or a shared screen (`a=content:slides`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoStream {
    Camera,
    Content,
}

/// The first video section of the given stream, however its port is set:
/// the first without `a=content:slides` is the camera, the first with it
/// the content.
fn first_video_section(sdp: &SessionDescription, stream: VideoStream) -> Option<&MediaDescription> {
    sdp.media
        .iter()
        .filter(|m| m.media_type == MediaType::Video)
        .find(|m| forge_sdp::is_slides(m) == (stream == VideoStream::Content))
}

/// The first *active* video section of the given stream.
fn active_video_section(
    sdp: &SessionDescription,
    stream: VideoStream,
) -> Option<&MediaDescription> {
    let v = forge_sdp::video_sections(sdp);
    match stream {
        VideoStream::Camera => v.camera,
        VideoStream::Content => v.content,
    }
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

/// SDES crypto lines of the camera's video section (each section carries
/// its own key; the audio section's never applies to video, and the
/// content section's is its own again).
pub fn parse_video_sdes_crypto(sdp_body: &str) -> Vec<SdesCryptoLine> {
    parse_stream_sdes_crypto(sdp_body, VideoStream::Camera)
}

/// SDES crypto lines of the shared screen's section (`a=content:slides`).
pub fn parse_content_sdes_crypto(sdp_body: &str) -> Vec<SdesCryptoLine> {
    parse_stream_sdes_crypto(sdp_body, VideoStream::Content)
}

fn parse_stream_sdes_crypto(sdp_body: &str, stream: VideoStream) -> Vec<SdesCryptoLine> {
    match SessionDescription::parse(sdp_body) {
        Ok(sdp) => first_video_section(&sdp, stream)
            .map(crypto_of_section)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
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

// ── Offers ──────────────────────────────────────────────────────────────

/// The SRTP half of an outbound offer: the key we put in our `a=crypto`
/// line and, once the answer carries the peer's, the context for the leg.
/// The answerer-shaped path (peer's key first, ours generated in reply)
/// lives with the conference server; this is the offerer's.
#[derive(Debug, Clone)]
pub struct SrtpOffer {
    local: forge_rtp::srtp::SrtpKeyMaterial,
    /// The `a=crypto` value for the section this key protects.
    pub sdp: SrtpSdpContext,
}

impl SrtpOffer {
    /// The suite every offer uses.
    pub const SUITE: &'static str = "AES_CM_128_HMAC_SHA1_80";

    /// Fresh key material for one media section.
    pub fn generate() -> Result<Self> {
        let (local, b64) = generate_srtp_key_material()?;
        Ok(Self {
            local,
            sdp: SrtpSdpContext {
                local_key_b64: format!("inline:{b64}"),
                suite: Self::SUITE.to_string(),
                tag: 1,
            },
        })
    }

    /// Our key for answering a peer's offered `a=crypto`: fresh, under the
    /// offer's tag (RFC 4568 §5.1.2). Only the suite every offer here uses
    /// is taken; [`accept`](Self::accept) then installs the peer's key.
    pub fn answering(offered: &SdesCryptoLine) -> Result<Self> {
        if !offered.suite.eq_ignore_ascii_case(Self::SUITE) {
            return Err(Error::Sdp(format!(
                "the offer's SRTP suite {} is not {}",
                offered.suite,
                Self::SUITE
            )));
        }
        let mut ours = Self::generate()?;
        ours.sdp.tag = offered.tag;
        Ok(ours)
    }

    /// Install the peer's key from the answer's `a=crypto` line for this
    /// section. The answer must keep our suite (RFC 4568 §5.1.2).
    pub fn accept(self, answer: &SdesCryptoLine) -> Result<forge_rtp::srtp::SrtpContext> {
        if !answer.suite.eq_ignore_ascii_case(&self.sdp.suite) {
            return Err(Error::Sdp(format!(
                "answer changed the SRTP suite from {} to {}",
                self.sdp.suite, answer.suite
            )));
        }
        let (key, salt, profile) = decode_sdes_key_params(&answer.key_params, &answer.suite)?;
        let remote = forge_rtp::srtp::SrtpKeyMaterial::new(key, salt, profile)
            .map_err(|e| Error::Sdp(format!("invalid SRTP key material in answer: {e}")))?;
        let mut ctx = forge_rtp::srtp::SrtpContext::new();
        ctx.set_local_key(self.local);
        ctx.set_remote_key(remote);
        Ok(ctx)
    }
}

/// The audio section of an outbound offer.
#[derive(Debug, Clone)]
pub struct AudioOfferSection<'a> {
    /// Our RTP port.
    pub rtp_port: u16,
    /// Codecs offered, in preference order.
    pub codecs: &'a [SdpCodecInfo],
    /// telephone-event `(payload type, clock rate)`, if offered.
    pub telephone_event: Option<(u8, u32)>,
    /// `a=crypto` for the section (`RTP/SAVP`); `None` offers plain RTP.
    pub srtp: Option<&'a SrtpSdpContext>,
    /// `a=ptime`, if stated.
    pub ptime_ms: Option<u32>,
}

/// The video section of an outbound offer: one codec at one payload type.
#[derive(Debug, Clone)]
pub struct VideoOfferSection<'a> {
    /// Our RTP port.
    pub rtp_port: u16,
    /// The payload type offered.
    pub payload_type: u8,
    /// The codec.
    pub codec: VideoCodec,
    /// `a=fmtp` parameters for it, if any (H.264 profile and mode).
    pub fmtp: Option<&'a str>,
    /// Offer `nack`, `nack pli`, `ccm fir` and `goog-remb` (and the AVPF
    /// profile); without it the section is plain AVP.
    pub feedback: bool,
    /// `a=crypto` for the section; `None` offers plain RTP.
    pub srtp: Option<&'a SrtpSdpContext>,
    /// Simulcast this section sends (RFC 8853): `a=rid:<id> send` per
    /// stream, `a=simulcast:send …` and the rid header extension under
    /// the layers' id. The gateway passes on what a browser agreed to send
    /// it, under the browser's own id, so the packets cross unchanged.
    pub simulcast: Option<&'a forge_sdp::SimulcastLayers>,
    /// Offer transport-cc under this extension id (`a=extmap` and
    /// `a=rtcp-fb:<pt> transport-cc`, `docs/TRANSPORT_CC.md` §3.4); needs
    /// `feedback`.
    pub transport_cc: Option<u8>,
}

/// The `a=label` an offered content section carries (RFC 4574), by which
/// BFCP's `a=floorid … mstrm:` names the stream a floor controls.
pub const CONTENT_LABEL: &str = "2";

/// Everything an offer needs.
#[derive(Debug, Clone)]
pub struct OfferOptions<'a> {
    /// The address on `o=` and `c=`.
    pub server_ip: &'a str,
    /// The `o=` session id.
    pub origin_id: &'a str,
    /// The `s=` line.
    pub session_name: &'a str,
    /// The audio section.
    pub audio: AudioOfferSection<'a>,
    /// The camera's video section, if offered.
    pub video: Option<VideoOfferSection<'a>>,
    /// A shared screen's section (`a=content:slides`, `a=label`), if
    /// offered. Only beside `video`.
    pub content: Option<VideoOfferSection<'a>>,
}

/// Construct an SDP offer with an audio section and, optionally, a video
/// section, each with its own SDES key when asked. This is the offer an
/// outbound leg sends — the gateway's leg to a conference server — and
/// the answer it gets back is read with [`parse_sdp_codecs`],
/// [`parse_video_offer`], [`parse_sdes_crypto`] and
/// [`parse_video_sdes_crypto`] as any peer's SDP is.
pub fn construct_offer(opts: &OfferOptions<'_>) -> Result<String> {
    let a = &opts.audio;
    let mut audio = MediaDescription::audio(a.rtp_port);
    audio.protocol = if a.srtp.is_some() {
        Protocol::RtpSavp
    } else {
        Protocol::RtpAvp
    };
    if let Some(ctx) = a.srtp {
        audio = audio
            .add_attribute("crypto", &ctx.crypto_value())
            .map_err(sdp_err)?;
    }
    if a.codecs.is_empty() {
        return Err(Error::Sdp("an offer needs at least one audio codec".into()));
    }
    for c in a.codecs {
        audio = audio.add_format(c.payload_type).map_err(sdp_err)?;
        audio = audio
            .add_rtpmap(c.payload_type, &c.encoding_name, c.clock_rate, None)
            .map_err(sdp_err)?;
    }
    if let Some((te, rate)) = a.telephone_event {
        audio = audio.add_format(te).map_err(sdp_err)?;
        audio = audio
            .add_rtpmap(te, "telephone-event", rate, None)
            .map_err(sdp_err)?;
        audio = audio
            .add_attribute("fmtp", &format!("{te} 0-16"))
            .map_err(sdp_err)?;
    }
    if let Some(ptime) = a.ptime_ms {
        audio = audio
            .add_attribute("ptime", &ptime.to_string())
            .map_err(sdp_err)?;
    }
    audio = audio.add_property("sendrecv").map_err(sdp_err)?;
    let mut sections = vec![audio];

    for (v, stream) in opts
        .video
        .iter()
        .map(|v| (v, VideoStream::Camera))
        .chain(opts.content.iter().map(|c| (c, VideoStream::Content)))
    {
        let mut video = MediaDescription::video(v.rtp_port);
        video.protocol = match (v.srtp.is_some(), v.feedback) {
            (true, true) => Protocol::RtpSavpf,
            (true, false) => Protocol::RtpSavp,
            (false, true) => Protocol::RtpAvpf,
            (false, false) => Protocol::RtpAvp,
        };
        if let Some(ctx) = v.srtp {
            video = video
                .add_attribute("crypto", &ctx.crypto_value())
                .map_err(sdp_err)?;
        }
        let pt = v.payload_type;
        video = video.add_format(pt).map_err(sdp_err)?;
        video = video
            .add_rtpmap(pt, v.codec.sdp_name(), VideoCodec::CLOCK_RATE, None)
            .map_err(sdp_err)?;
        if let Some(fmtp) = v.fmtp {
            video = video
                .add_attribute("fmtp", &format!("{pt} {fmtp}"))
                .map_err(sdp_err)?;
        }
        if v.feedback {
            for fb in ["nack", "nack pli", "ccm fir", "goog-remb"] {
                video = video
                    .add_attribute("rtcp-fb", &format!("{pt} {fb}"))
                    .map_err(sdp_err)?;
            }
            if let Some(id) = v.transport_cc {
                video = video
                    .add_attribute("rtcp-fb", &format!("{pt} transport-cc"))
                    .map_err(sdp_err)?
                    .add_attribute(
                        "extmap",
                        &format!("{id} {}", forge_rtp::transport_cc::TRANSPORT_CC_URI),
                    )
                    .map_err(sdp_err)?;
            }
        }
        if stream == VideoStream::Content {
            forge_sdp::set_content(&mut video, &[forge_sdp::Content::Slides]);
            forge_sdp::set_label(&mut video, CONTENT_LABEL);
        }
        if let Some(layers) = v.simulcast {
            forge_sdp::write_simulcast(&mut video, forge_sdp::RidDirection::Send, layers);
        }
        video = video.add_property("sendrecv").map_err(sdp_err)?;
        sections.push(video);
    }

    serialize_answer(opts.server_ip, opts.origin_id, opts.session_name, sections)
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

/// The remote video RTP endpoint of the camera's section (the first
/// active video section that is not `a=content:slides`).
pub fn remote_video_endpoint(sdp_body: &str) -> Option<SocketAddr> {
    if let Ok(sdp) = SessionDescription::parse(sdp_body) {
        return active_video_section(&sdp, VideoStream::Camera)
            .and_then(|m| section_endpoint(&sdp, m));
    }
    remote_endpoint(sdp_body, MediaType::Video)
}

/// The remote RTP endpoint of the shared screen's section.
pub fn remote_content_endpoint(sdp_body: &str) -> Option<SocketAddr> {
    let sdp = SessionDescription::parse(sdp_body).ok()?;
    active_video_section(&sdp, VideoStream::Content).and_then(|m| section_endpoint(&sdp, m))
}

/// A section's own `c=`, or the session's.
fn section_endpoint(sdp: &SessionDescription, m: &MediaDescription) -> Option<SocketAddr> {
    if m.port == 0 {
        return None;
    }
    let conn = m.connection.as_ref().or(sdp.connection.as_ref())?;
    let ip: IpAddr = conn.connection_address.as_str().parse().ok()?;
    Some(SocketAddr::new(ip, m.port))
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

/// What a peer offered on one video section.
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
    /// The section's `a=label` (RFC 4574), if any.
    pub label: Option<String>,
    /// Which stream the section carries.
    pub stream: VideoStream,
    /// The peer offers to send on it (`sendrecv` or `sendonly`): for a
    /// shared screen, that it is sharing or about to.
    pub sends: bool,
    /// The transport-cc extension's id, when the section carries both it
    /// and `transport-cc` feedback.
    pub transport_cc: Option<u8>,
}

/// The video sections of an offer a conference cares about (RFC 4796):
/// the camera, and the shared screen when the endpoint can send one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VideoSections {
    /// The first active video section without `a=content:slides`.
    pub camera: Option<VideoOffer>,
    /// The first active video section with it.
    pub content: Option<VideoOffer>,
}

fn video_offer_of(
    sdp: &SessionDescription,
    m: &MediaDescription,
    stream: VideoStream,
) -> VideoOffer {
    let codecs = m
        .video_codecs()
        .into_iter()
        .map(|(ci, codec)| VideoOfferCodec {
            payload_type: ci.payload_type,
            codec,
            fmtp: m.fmtp_for(ci.payload_type).map(|f| f.params.to_string()),
        })
        .collect();
    VideoOffer {
        port: m.port,
        protocol: m.protocol.to_string(),
        codecs,
        remote: section_endpoint(sdp, m),
        feedback: !m.rtcp_fb_iter().is_empty(),
        crypto: !crypto_of_section(m).is_empty(),
        label: forge_sdp::label(m).map(str::to_string),
        stream,
        sends: matches!(forge_sdp::direction_of(m), "sendrecv" | "sendonly"),
        transport_cc: transport_cc_ext(sdp, m).filter(|_| {
            m.rtcp_fb_iter()
                .iter()
                .any(|f| f.kind == "transport-cc" && f.param.is_none())
        }),
    }
}

/// Parse the camera's video section of an offer — the first active video
/// section that is not a shared screen — or `None` when the offer has no
/// such section (or only a disabled one). A device that shares a screen
/// and has no camera offers only a `slides` section, which this does not
/// return: see [`parse_video_sections`].
pub fn parse_video_offer(sdp_body: &str) -> Option<VideoOffer> {
    parse_video_sections(sdp_body).camera
}

/// Parse both video sections of an offer: the camera and, when the
/// endpoint can share a screen, the content (`a=content:slides`). Any
/// further video section is nobody's and is rejected by the answer.
pub fn parse_video_sections(sdp_body: &str) -> VideoSections {
    let Ok(sdp) = SessionDescription::parse(sdp_body) else {
        return VideoSections::default();
    };
    let v = forge_sdp::video_sections(&sdp);
    VideoSections {
        camera: v
            .camera
            .map(|m| video_offer_of(&sdp, m, VideoStream::Camera)),
        content: v
            .content
            .map(|m| video_offer_of(&sdp, m, VideoStream::Content)),
    }
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

/// An anchored offer's video made secure: the camera's and a shared
/// screen's sections, when `keys` has one for them, `RTP/SAVP(F)` with the
/// node's key, any other key in them dropped. Audio is [`with_sdes`]'s.
pub fn with_video_sdes(sdp: &str, keys: AnchorKeys<'_>) -> String {
    let kinds = classify_sections(sdp);
    let (head, sections) = split_sections(sdp);
    if sections.len() != kinds.len() {
        return sdp.to_string();
    }
    let (mut camera_seen, mut content_seen) = (false, false);
    let mut out = head;
    for (section, kind) in sections.iter().zip(kinds) {
        let key = match kind {
            SectionKind::Camera if !camera_seen => {
                camera_seen = true;
                keys.video
            }
            SectionKind::Content if !content_seen => {
                content_seen = true;
                keys.content
            }
            _ => None,
        };
        let port = section
            .lines()
            .next()
            .and_then(|m| m.split_whitespace().nth(1))
            .and_then(|p| p.parse::<u16>().ok());
        match (key, port) {
            (Some(key), Some(port)) if port != 0 => {
                let mut keyed = String::new();
                for line in section.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("m=") {
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        keyed.push_str(&format!(
                            "m={} {} {}",
                            parts[0],
                            parts[1],
                            secure_protocol(parts.get(2).copied().unwrap_or("RTP/AVP"))
                        ));
                        for part in parts.iter().skip(3) {
                            keyed.push(' ');
                            keyed.push_str(part);
                        }
                        keyed.push_str("\r\n");
                        keyed.push_str(&format!("a=crypto:{}\r\n", key.crypto_value()));
                        continue;
                    }
                    if trimmed.starts_with("a=crypto:") {
                        continue;
                    }
                    keyed.push_str(trimmed);
                    keyed.push_str("\r\n");
                }
                out.push_str(&keyed);
            }
            _ => out.push_str(section),
        }
    }
    out
}

/// An anchored offer made secure: the first audio section `RTP/SAVP` with
/// `srtp`'s key, any other key in it dropped. What a leg the node keys
/// itself — a softphone registered over TLS — is offered.
pub fn with_sdes(sdp: &str, srtp: &SrtpSdpContext) -> String {
    let mut out = String::with_capacity(sdp.len() + 96);
    let (mut in_audio, mut audio_seen) = (false, false);
    for line in sdp.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("m=") {
            in_audio = false;
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.first() == Some(&"audio") && parts.len() >= 3 && !audio_seen {
                audio_seen = true;
                in_audio = true;
                out.push_str(&format!(
                    "m=audio {} {}",
                    parts[1],
                    secure_protocol(parts[2])
                ));
                for p in &parts[3..] {
                    out.push(' ');
                    out.push_str(p);
                }
                out.push_str("\r\n");
                out.push_str(&format!("a=crypto:{}\r\n", srtp.crypto_value()));
                continue;
            }
        }
        if in_audio && trimmed.starts_with("a=crypto:") {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out
}

/// The node's own SDES key in an SDP it made for a leg — the answer the
/// media backend gave an encrypted caller — to answer that leg with again.
/// `None` when the SDP carries none (the leg is plain).
pub fn srtp_context_of(sdp: &str) -> Option<SrtpSdpContext> {
    let line = parse_sdes_crypto(sdp).into_iter().next()?;
    Some(SrtpSdpContext {
        local_key_b64: line.key_params,
        suite: line.suite,
        tag: line.tag,
    })
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
    /// How many simulcast streams to take from an offer that sends
    /// several (RFC 8853), in the offer's order; 0 takes none, and the
    /// sender then sends one.
    pub simulcast: usize,
    /// Keep transport-cc when the offer has it on this section (the
    /// transport-wide sequence extension and `a=rtcp-fb:<pt>
    /// transport-cc`, `docs/TRANSPORT_CC.md` §3.3); `false` leaves both
    /// out, as an answer always did.
    pub transport_cc: bool,
}

/// Everything an answer needs besides the offer.
#[derive(Debug, Clone)]
pub struct AnswerOptions<'a> {
    pub server_ip: &'a str,
    pub audio_rtp_port: u16,
    pub audio_srtp: Option<&'a SrtpSdpContext>,
    /// How to answer the camera's section; `None` rejects it (port 0).
    pub video: Option<VideoAnswer<'a>>,
    /// How to answer a shared screen's section (`a=content:slides`), on
    /// its own port with its own key; `None` rejects it.
    pub content: Option<VideoAnswer<'a>>,
    /// How to answer a BFCP section (RFC 8856), as the floor control
    /// server; `None` rejects it.
    pub bfcp: Option<BfcpAnswer<'a>>,
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
    /// The section's `a=label`, repeated in the answer (RFC 4574).
    pub label: Option<String>,
    /// The simulcast streams the answer took, when the offer sent several
    /// and the answer asked for them.
    pub simulcast: Option<forge_sdp::SimulcastLayers>,
    /// The transport-wide sequence extension's id, when the answer kept
    /// transport-cc: the peer numbers what it sends under it and wants
    /// feedback, and reads ours.
    pub transport_cc: Option<u8>,
}

/// An answer and what it agreed.
#[derive(Debug, Clone)]
pub struct Answer {
    pub sdp: String,
    /// `Some` when the camera's section was accepted.
    pub video: Option<NegotiatedVideo>,
    /// `Some` when a shared screen's section was accepted.
    pub content: Option<NegotiatedVideo>,
    /// `Some` when a BFCP section was answered (RFC 8856).
    pub bfcp: Option<NegotiatedBfcp>,
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
    offer: &SessionDescription,
    m: &MediaDescription,
    v: &VideoAnswer<'_>,
    remote: Option<SocketAddr>,
) -> (MediaDescription, Option<NegotiatedVideo>) {
    let Some((ci, codec)) = choose_allowed_video_codec(m, v.codecs) else {
        return (reject_section(m), None);
    };
    let mut sec = answer_video(m, &ci, v.rtp_port, "sendrecv");
    // Simulcast taken: the streams the offer sends, received (RFC 8853
    // §5.3), under the offer's rid extension. The rids are answered
    // without the offer's restrictions: a `pt=` naming a payload type
    // this answer dropped would be wrong, and none narrows what we take.
    let simulcast = (v.simulcast >= 2)
        .then(|| forge_sdp::simulcast_layers(offer, m, forge_sdp::RidDirection::Send, v.simulcast))
        .flatten()
        .map(|mut layers| {
            for rid in &mut layers.rids {
                rid.restrictions = Default::default();
            }
            layers
        });
    if let Some(layers) = &simulcast {
        forge_sdp::write_simulcast(&mut sec, forge_sdp::RidDirection::Recv, layers);
    }
    // Transport-cc, when we carry it and the offer asks for it on the
    // chosen codec: its extension under the offer's id, and the feedback.
    let transport_cc = v
        .transport_cc
        .then(|| transport_cc_ext(offer, m))
        .flatten()
        .filter(|_| {
            m.rtcp_fb_for(ci.payload_type)
                .iter()
                .any(|f| f.kind == "transport-cc" && f.param.is_none())
        });
    if let Some(id) = transport_cc {
        sec.attributes.push(Attribute::Value {
            name: "extmap".into(),
            value: format!("{id} {}", forge_rtp::transport_cc::TRANSPORT_CC_URI).into(),
        });
        sec.attributes.push(Attribute::Value {
            name: "rtcp-fb".into(),
            value: format!("{} transport-cc", ci.payload_type).into(),
        });
    }
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
        label: forge_sdp::label(m).map(str::to_string),
        simulcast,
        transport_cc,
    };
    (sec, Some(negotiated))
}

/// The id the offer gives the transport-wide sequence extension on `m`,
/// or at session level.
fn transport_cc_ext(offer: &SessionDescription, m: &MediaDescription) -> Option<u8> {
    m.attributes
        .iter()
        .chain(offer.attributes.iter())
        .find_map(|a| match a {
            Attribute::Value { name, value } if name.as_str() == "extmap" => {
                let mut parts = value.split_whitespace();
                let id = parts.next()?.split('/').next()?.parse::<u8>().ok()?;
                (parts.next()? == forge_rtp::transport_cc::TRANSPORT_CC_URI && id > 0).then_some(id)
            }
            _ => None,
        })
}

/// What the video sections of an answer agreed.
#[derive(Debug, Clone, Default)]
pub struct NegotiatedSections {
    pub video: Option<NegotiatedVideo>,
    pub content: Option<NegotiatedVideo>,
    pub bfcp: Option<NegotiatedBfcp>,
}

// ── BFCP (RFC 8856) ──────────────────────────────────────────────────────

/// What a BFCP section rides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BfcpTransport {
    /// `UDP/BFCP`: one message per datagram, version 2.
    Udp,
    /// `TCP/BFCP`: the client connects to the server, version 1.
    Tcp,
}

impl BfcpTransport {
    pub fn proto(self) -> &'static str {
        match self {
            BfcpTransport::Udp => "UDP/BFCP",
            BfcpTransport::Tcp => "TCP/BFCP",
        }
    }

    /// The `a=bfcpver` the transport implies (RFC 8856 §5.5).
    pub fn version(self) -> u8 {
        match self {
            BfcpTransport::Udp => 2,
            BfcpTransport::Tcp => 1,
        }
    }

    fn of(protocol: &Protocol) -> Option<BfcpTransport> {
        match protocol {
            Protocol::Other(p) if p.eq_ignore_ascii_case("UDP/BFCP") => Some(BfcpTransport::Udp),
            Protocol::Other(p) if p.eq_ignore_ascii_case("TCP/BFCP") => Some(BfcpTransport::Tcp),
            _ => None,
        }
    }
}

/// A BFCP section an endpoint offered (RFC 8856): the first active
/// `m=application … UDP/BFCP` or `TCP/BFCP`. TLS and DTLS variants are
/// not taken (they are rejected with port 0 like any other section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BfcpOffer {
    pub transport: BfcpTransport,
    pub port: u16,
    pub remote: Option<SocketAddr>,
    /// The roles the offerer is willing to take (`a=floorctrl`), as
    /// offered; empty means the RFC's default, client only.
    pub floorctrl: Vec<String>,
    /// `a=setup` on a TCP section; the offerer's default is `active`.
    pub setup: Option<String>,
    /// The versions offered (`a=bfcpver`).
    pub versions: Vec<u8>,
}

impl BfcpOffer {
    /// Whether this server can answer it: the offerer is willing to be
    /// the client (it offered `c-only` or `c-s`, or nothing), and over
    /// TCP it will connect to us (`active` or `actpass`).
    pub fn answerable(&self) -> bool {
        let client_ok =
            self.floorctrl.is_empty() || self.floorctrl.iter().any(|f| f == "c-only" || f == "c-s");
        let setup_ok = match self.transport {
            BfcpTransport::Udp => true,
            BfcpTransport::Tcp => matches!(
                self.setup.as_deref().unwrap_or("active"),
                "active" | "actpass"
            ),
        };
        let version_ok =
            self.versions.is_empty() || self.versions.contains(&self.transport.version());
        client_ok && setup_ok && version_ok
    }
}

fn attr_value<'a>(m: &'a MediaDescription, name: &str) -> Option<&'a str> {
    m.attributes.iter().find_map(|a| match a {
        Attribute::Value { name: n, value } if n == name => Some(value.as_str()),
        _ => None,
    })
}

fn bfcp_offer_of(sdp: &SessionDescription, m: &MediaDescription) -> Option<BfcpOffer> {
    if m.media_type != MediaType::Application || m.port == 0 {
        return None;
    }
    let transport = BfcpTransport::of(&m.protocol)?;
    Some(BfcpOffer {
        transport,
        port: m.port,
        remote: section_endpoint(sdp, m),
        floorctrl: attr_value(m, "floorctrl")
            .map(|v| v.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        setup: attr_value(m, "setup").map(str::to_string),
        versions: attr_value(m, "bfcpver")
            .map(|v| {
                v.split_whitespace()
                    .filter_map(|t| t.parse().ok())
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The BFCP section of an offer, if it has one this server could take.
pub fn parse_bfcp_offer(sdp_body: &str) -> Option<BfcpOffer> {
    let sdp = SessionDescription::parse(sdp_body).ok()?;
    sdp.media.iter().find_map(|m| bfcp_offer_of(&sdp, m))
}

/// How to answer a BFCP section: this server is the floor control server
/// (`a=floorctrl:s-only`) and assigns the ids the client will use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BfcpAnswer<'a> {
    /// Our port, for either transport (the same socket serves both).
    pub port: u16,
    pub conference_id: u32,
    pub user_id: u16,
    pub floor_id: u16,
    /// The `a=label` of the content section the floor controls
    /// (`a=floorid:<floor> mstrm:<label>`).
    pub label: &'a str,
}

/// What a BFCP section of an answer agreed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedBfcp {
    pub transport: BfcpTransport,
    pub conference_id: u32,
    pub user_id: u16,
    pub floor_id: u16,
    /// Where the client's messages come from over UDP, as offered.
    pub remote: Option<SocketAddr>,
}

fn bfcp_answer_section(
    m: &MediaDescription,
    offer: &BfcpOffer,
    b: &BfcpAnswer<'_>,
) -> MediaDescription {
    let mut sec = reject_section(m);
    sec.port = b.port;
    let mut attrs = vec![
        ("floorctrl", "s-only".to_string()),
        ("confid", b.conference_id.to_string()),
        ("userid", b.user_id.to_string()),
        ("floorid", format!("{} mstrm:{}", b.floor_id, b.label)),
        ("bfcpver", offer.transport.version().to_string()),
    ];
    if offer.transport == BfcpTransport::Tcp {
        // The client connects to us (RFC 4145).
        attrs.push(("setup", "passive".to_string()));
        attrs.push(("connection", "new".to_string()));
    }
    for (name, value) in attrs {
        sec.attributes.push(Attribute::Value {
            name: name.into(),
            value: value.into(),
        });
    }
    sec
}

/// One answer section per offered section, in order (RFC 3264 §6): the
/// first active audio section through `audio`, the camera's section (the
/// first active video section that is not `a=content:slides`) through
/// `video`, a shared screen's (the first that is) through `content`, each
/// rejected when `None`, and everything else rejected with port 0.
pub fn answer_sections(
    offer: &SessionDescription,
    mut audio: impl FnMut(&MediaDescription) -> Result<MediaDescription>,
    video: Option<&VideoAnswer<'_>>,
    content: Option<&VideoAnswer<'_>>,
    bfcp: Option<&BfcpAnswer<'_>>,
) -> Result<(Vec<MediaDescription>, NegotiatedSections)> {
    let mut sections = Vec::with_capacity(offer.media.len());
    let mut audio_done = false;
    let mut camera_done = false;
    let mut content_done = false;
    let mut bfcp_done = false;
    let mut negotiated = NegotiatedSections::default();
    for m in &offer.media {
        let section = match m.media_type {
            // The floor control section (RFC 8856), answered as the
            // server when asked to and the offer lets us be one.
            MediaType::Application if !bfcp_done && bfcp_offer_of(offer, m).is_some() => {
                bfcp_done = true;
                let offered = bfcp_offer_of(offer, m).expect("checked");
                match bfcp {
                    Some(b) if offered.answerable() => {
                        negotiated.bfcp = Some(NegotiatedBfcp {
                            transport: offered.transport,
                            conference_id: b.conference_id,
                            user_id: b.user_id,
                            floor_id: b.floor_id,
                            remote: offered.remote,
                        });
                        bfcp_answer_section(m, &offered, b)
                    }
                    _ => reject_section(m),
                }
            }
            MediaType::Audio if !audio_done && m.port != 0 => {
                audio_done = true;
                audio(m)?
            }
            MediaType::Video if m.port != 0 && forge_sdp::is_slides(m) && !content_done => {
                content_done = true;
                match content {
                    Some(c) => {
                        let (sec, neg) =
                            video_answer_section(offer, m, c, section_endpoint(offer, m));
                        negotiated.content = neg;
                        sec
                    }
                    None => reject_section(m),
                }
            }
            MediaType::Video if m.port != 0 && !forge_sdp::is_slides(m) && !camera_done => {
                camera_done = true;
                match video {
                    Some(v) => {
                        let (sec, neg) =
                            video_answer_section(offer, m, v, section_endpoint(offer, m));
                        negotiated.video = neg;
                        sec
                    }
                    None => reject_section(m),
                }
            }
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
    let (sections, negotiated) = answer_sections(
        &parsed,
        |m| audio_answer_section(m, opts),
        opts.video.as_ref(),
        opts.content.as_ref(),
        opts.bfcp.as_ref(),
    )?;
    let sdp = serialize_answer(opts.server_ip, "0", "FCP Media Server", sections)?;
    Ok(Answer {
        sdp,
        video: negotiated.video,
        content: negotiated.content,
        bfcp: negotiated.bfcp,
    })
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
        content: None,
        bfcp: None,
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
    /// Our port for the camera's video section; `None` disables it (port
    /// 0), since nothing would relay it.
    pub video: Option<u16>,
    /// Our port for a shared screen's section (`a=content:slides`); `None`
    /// disables it. A second video section of either kind is always
    /// disabled: one port carries one stream.
    pub content: Option<u16>,
}

/// What each `m=` line of an SDP is, for the anchoring rewriter, which
/// works line by line and cannot see a section's `a=content` from its
/// `m=` line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Audio,
    Camera,
    Content,
    Other,
}

/// Classify every section of an SDP in order; a body the parser rejects
/// is classified by its `m=` lines alone (every video section a camera).
fn classify_sections(sdp_body: &str) -> Vec<SectionKind> {
    if let Ok(sdp) = SessionDescription::parse(sdp_body) {
        return sdp
            .media
            .iter()
            .map(|m| match m.media_type {
                MediaType::Audio => SectionKind::Audio,
                MediaType::Video if forge_sdp::is_slides(m) => SectionKind::Content,
                MediaType::Video => SectionKind::Camera,
                _ => SectionKind::Other,
            })
            .collect();
    }
    sdp_body
        .lines()
        .filter_map(|l| l.trim().strip_prefix("m="))
        .map(|rest| match rest.split_whitespace().next() {
            Some("audio") => SectionKind::Audio,
            Some("video") => SectionKind::Camera,
            _ => SectionKind::Other,
        })
        .collect()
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

/// The SDES-secured profile's plain counterpart: an anchored leg the node
/// does not key is offered RTP in the clear.
fn plain_protocol(protocol: &str) -> String {
    match protocol {
        "RTP/SAVP" => "RTP/AVP".to_string(),
        "RTP/SAVPF" => "RTP/AVPF".to_string(),
        other => other.to_string(),
    }
}

/// The node's SDES keys for the sections of an SDP it sends a leg, one
/// per section: `None` sends that section plain.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnchorKeys<'a> {
    pub audio: Option<&'a SrtpSdpContext>,
    pub video: Option<&'a SrtpSdpContext>,
    pub content: Option<&'a SrtpSdpContext>,
}

impl<'a> AnchorKeys<'a> {
    /// Audio's key alone.
    pub fn audio(audio: Option<&'a SrtpSdpContext>) -> Self {
        Self {
            audio,
            ..Self::default()
        }
    }
}

/// Rewrite an SDP so every section points at the server: every `c=` line
/// becomes `server_ip`; the first audio section takes `ports.audio`, the
/// camera's video section `ports.video` and a shared screen's
/// `ports.content`, a video section disabled with port 0 when its port is
/// `None`. Each section the node carries is secure with the node's own key
/// for it from `keys` (`RTP/SAVP` and an `a=crypto`), or plain; the other
/// leg's crypto lines are dropped from every section, so one leg's key
/// never reaches the other. A second section of any kind, and every other
/// section, is disabled — one port carries one stream. A disabled section
/// keeps its formats and attributes, as RFC 3264 requires.
pub fn rewrite_for_anchoring(
    original_sdp: &str,
    server_ip: &str,
    ports: AnchorPorts,
    keys: AnchorKeys<'_>,
) -> String {
    let kinds = classify_sections(original_sdp);
    let mut result = String::new();
    let mut section_index = 0usize;
    let mut audio_seen = false;
    let mut camera_seen = false;
    let mut content_seen = false;

    for line in original_sdp.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("m=") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let kind = parts[0];
                let section = kinds
                    .get(section_index)
                    .copied()
                    .unwrap_or(SectionKind::Other);
                section_index += 1;
                // The node terminates each leg's media: a section it
                // carries is secure with its key, or plain — never the
                // other leg's profile and key passed through.
                let carried = match section {
                    SectionKind::Audio if !audio_seen => {
                        audio_seen = true;
                        Some((ports.audio, keys.audio))
                    }
                    SectionKind::Camera if !camera_seen && ports.video.is_some() => {
                        camera_seen = true;
                        ports.video.map(|p| (p, keys.video))
                    }
                    SectionKind::Content if !content_seen && ports.content.is_some() => {
                        content_seen = true;
                        ports.content.map(|p| (p, keys.content))
                    }
                    _ => None,
                };
                let (port, protocol, key) = match carried {
                    Some((port, Some(key))) => (port, secure_protocol(parts[2]), Some(key)),
                    Some((port, None)) => (port, plain_protocol(parts[2]), None),
                    None => (0, parts[2].to_string(), None),
                };
                result.push_str(&format!("m={kind} {port} {protocol}"));
                for part in &parts[3..] {
                    result.push(' ');
                    result.push_str(part);
                }
                result.push_str("\r\n");
                if let Some(key) = key {
                    result.push_str(&format!("a=crypto:{}\r\n", key.crypto_value()));
                }
                continue;
            }
        }

        // The other leg's key never crosses the node: ours replaces it, or
        // the section goes without.
        if trimmed.starts_with("a=crypto:") {
            continue;
        }

        // The origin names the node, not the leg it came from: the other
        // leg's address stays on its side. Its session id and version stay,
        // so a re-offer still counts up.
        if let Some(origin) = trimmed.strip_prefix("o=") {
            let parts: Vec<&str> = origin.split_whitespace().collect();
            if parts.len() == 6 && parts[3] == "IN" {
                let addrtype = if server_ip.contains(':') {
                    "IP6"
                } else {
                    "IP4"
                };
                result.push_str(&format!(
                    "o={} {} {} IN {addrtype} {server_ip}\r\n",
                    parts[0], parts[1], parts[2]
                ));
                continue;
            }
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
            content: None,
        },
        AnchorKeys::audio(srtp),
    )
}

// ── Relayed video ───────────────────────────────────────────────────────

/// Each video stream an SDP carries live: a section with a port.
pub fn active_streams(sdp_body: &str) -> Vec<VideoStream> {
    [VideoStream::Camera, VideoStream::Content]
        .into_iter()
        .filter(|s| stream_endpoint(sdp_body, *s).is_some())
        .collect()
}

/// The RTP address of a live video stream's section.
pub fn stream_endpoint(sdp_body: &str, stream: VideoStream) -> Option<SocketAddr> {
    match stream {
        VideoStream::Camera => remote_video_endpoint(sdp_body),
        VideoStream::Content => remote_content_endpoint(sdp_body),
    }
}

/// The SDES crypto lines of a video stream's section.
pub fn stream_sdes_crypto(sdp_body: &str, stream: VideoStream) -> Vec<SdesCryptoLine> {
    parse_stream_sdes_crypto(sdp_body, stream)
}

/// Whether a live video stream's section puts RTCP on its RTP port
/// (`a=rtcp-mux`, RFC 5761).
pub fn stream_rtcp_mux(sdp_body: &str, stream: VideoStream) -> bool {
    let Ok(sdp) = SessionDescription::parse(sdp_body) else {
        return false;
    };
    active_video_section(&sdp, stream).is_some_and(|m| {
        m.attributes
            .iter()
            .any(|a| matches!(a, Attribute::Property(p) if p == "rtcp-mux"))
    })
}

/// An SDP cut at its `m=` lines: the session part, then each section's
/// lines, every line ended with CRLF.
fn split_sections(sdp: &str) -> (String, Vec<String>) {
    let mut head = String::new();
    let mut sections: Vec<String> = Vec::new();
    for line in sdp.lines() {
        if line.trim_start().starts_with("m=") {
            sections.push(String::new());
        }
        let out = sections.last_mut().unwrap_or(&mut head);
        out.push_str(line.trim_end_matches('\r'));
        out.push_str("\r\n");
    }
    (head, sections)
}

/// `sdp` with each live video section's direction — the camera's and a
/// shared screen's — set to `direction`, one added where a section had
/// none. Audio and turned-down sections are left alone: the node answers
/// a hold's video `inactive` while its audio answers as it always has.
pub fn with_video_direction(sdp: &str, direction: &str) -> String {
    let is_direction = |l: &str| {
        matches!(
            l.trim(),
            "a=sendrecv" | "a=sendonly" | "a=recvonly" | "a=inactive"
        )
    };
    let (head, sections) = split_sections(sdp);
    let mut out = head;
    for section in sections {
        let mut lines = section.lines();
        let m = lines.next().unwrap_or_default();
        let live_video =
            m.starts_with("m=video") && m.split_whitespace().nth(1).is_some_and(|p| p != "0");
        if !live_video {
            out.push_str(&section);
            continue;
        }
        out.push_str(m);
        out.push_str("\r\n");
        for line in lines.filter(|l| !is_direction(l)) {
            out.push_str(line);
            out.push_str("\r\n");
        }
        out.push_str(&format!("a={direction}\r\n"));
    }
    out
}

/// An answer's live video sections, the camera's and a shared screen's,
/// made to send and never receive: a webinar's attendee watches and is
/// not watched (FCP `docs/WEBINARS.md` §2.3). A section the answer sent
/// on (`sendrecv`, `sendonly`) becomes `sendonly`; one it only received on
/// (`recvonly`, the offer sending only) becomes `inactive`. Audio and
/// turned-down sections are left alone.
pub fn with_video_sending_only(answer: &str) -> String {
    let direction_of_line = |l: &str| match l.trim() {
        "a=sendrecv" => Some("sendrecv"),
        "a=sendonly" => Some("sendonly"),
        "a=recvonly" => Some("recvonly"),
        "a=inactive" => Some("inactive"),
        _ => None,
    };
    let (head, sections) = split_sections(answer);
    let mut out = head;
    for section in sections {
        let mut lines = section.lines();
        let m = lines.next().unwrap_or_default();
        let live_video =
            m.starts_with("m=video") && m.split_whitespace().nth(1).is_some_and(|p| p != "0");
        if !live_video {
            out.push_str(&section);
            continue;
        }
        let rest: Vec<&str> = lines.collect();
        let was = rest
            .iter()
            .find_map(|l| direction_of_line(l))
            .unwrap_or("sendrecv");
        let now = match was {
            "sendrecv" | "sendonly" => "sendonly",
            _ => "inactive",
        };
        out.push_str(m);
        out.push_str("\r\n");
        for line in rest.iter().filter(|l| direction_of_line(l).is_none()) {
            out.push_str(line);
            out.push_str("\r\n");
        }
        out.push_str(&format!("a={now}\r\n"));
    }
    out
}

/// One section pointed at the node: its port, its connection line, and
/// the node's key or none, the other leg's crypto dropped.
fn anchor_section(
    section: &str,
    server_ip: &str,
    port: u16,
    key: Option<&SrtpSdpContext>,
) -> String {
    let mut out = String::with_capacity(section.len() + 96);
    for line in section.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("m=") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3 {
                let protocol = match key {
                    Some(_) => secure_protocol(parts[2]),
                    None => plain_protocol(parts[2]),
                };
                out.push_str(&format!("m={} {port} {protocol}", parts[0]));
                for part in &parts[3..] {
                    out.push(' ');
                    out.push_str(part);
                }
                out.push_str("\r\n");
                if let Some(key) = key {
                    out.push_str(&format!("a=crypto:{}\r\n", key.crypto_value()));
                }
                continue;
            }
        }
        if trimmed.starts_with("a=crypto:") {
            continue;
        }
        if trimmed.starts_with("c=IN IP") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 3 {
                out.push_str(&format!("c=IN {} {}\r\n", parts[1], server_ip));
                continue;
            }
        }
        out.push_str(trimmed);
        out.push_str("\r\n");
    }
    out
}

/// The node's answer to a leg's `offer`, with the video it relays: each
/// video section of `answer` (the node's own, which turns video down) is
/// replaced by the other leg's answer to the same section, pointed at the
/// node on `ports` with `keys` — the other phone's codec, format
/// parameters, feedback and direction, as it chose them. A section the
/// other leg turned down, or one without a relay port, stays turned down.
///
/// The other leg was offered this leg's offer, section for section, so
/// the sections line up by position; if they do not, `answer` is returned
/// as it is and the call goes on without video.
pub fn splice_relayed_video(
    answer: &str,
    other_answer: &str,
    offer: &str,
    server_ip: &str,
    ports: AnchorPorts,
    keys: AnchorKeys<'_>,
) -> String {
    let kinds = classify_sections(offer);
    let (head, mut sections) = split_sections(answer);
    let (_, others) = split_sections(other_answer);
    if sections.len() != kinds.len() || others.len() != kinds.len() {
        return answer.to_string();
    }
    let (mut camera_seen, mut content_seen) = (false, false);
    for (i, kind) in kinds.iter().enumerate() {
        let relayed = match kind {
            SectionKind::Camera if !camera_seen => {
                camera_seen = true;
                ports.video.map(|p| (p, keys.video))
            }
            SectionKind::Content if !content_seen => {
                content_seen = true;
                ports.content.map(|p| (p, keys.content))
            }
            _ => None,
        };
        let Some((port, key)) = relayed else {
            continue;
        };
        let live = others[i]
            .lines()
            .next()
            .and_then(|m| m.split_whitespace().nth(1))
            .is_some_and(|p| p != "0");
        if live {
            sections[i] = anchor_section(&others[i], server_ip, port, key);
        }
    }
    let mut out = head;
    for section in sections {
        out.push_str(&section);
    }
    out
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

/// The same offer with each of `codecs` in its first audio section: those
/// it lacks are added after the ones it has, G.711, G.722 and G.729 at
/// their static payload types and Opus at a free dynamic one. A name not
/// known here is left out.
///
/// An anchored leg offers what the node can transcode to, not only what
/// the caller offered (`docs/DEEP_TESTING.md` D2d): [`filter_sdp_codecs`]
/// keeps codecs, so a codec the caller never offered has to be added
/// first.
pub fn ensure_audio_codecs(sdp_body: &str, codecs: &[&str]) -> String {
    let present = parse_sdp_codecs(sdp_body);
    let mut used: Vec<u8> = present.iter().map(|c| c.payload_type).collect();
    used.extend(extract_telephone_event_pt(sdp_body));
    let mut added: Vec<(u8, &str)> = Vec::new();
    for name in codecs {
        if present
            .iter()
            .any(|c| c.encoding_name.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let (pt, rtpmap) = match name.to_ascii_uppercase().as_str() {
            "PCMU" => (0, "PCMU/8000"),
            "PCMA" => (8, "PCMA/8000"),
            "G722" => (9, "G722/8000"),
            "G729" => (18, "G729/8000"),
            "OPUS" => match (96u8..=127).find(|pt| !used.contains(pt)) {
                Some(pt) => (pt, "opus/48000/2"),
                None => continue,
            },
            _ => continue,
        };
        if used.contains(&pt) {
            continue;
        }
        used.push(pt);
        added.push((pt, rtpmap));
    }
    if added.is_empty() {
        return sdp_body.to_string();
    }

    let rtpmaps: String = added
        .iter()
        .map(|(pt, map)| format!("a=rtpmap:{pt} {map}\r\n"))
        .collect();
    let mut result = String::new();
    let mut audio_seen = false;
    let mut in_audio = false;
    for line in sdp_body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("m=") {
            if in_audio {
                // The audio section ends here: its new rtpmaps close it.
                result.push_str(&rtpmaps);
                in_audio = false;
            }
            if trimmed.starts_with("m=audio ") && !audio_seen {
                audio_seen = true;
                in_audio = true;
                result.push_str(trimmed);
                for (pt, _) in &added {
                    result.push_str(&format!(" {pt}"));
                }
                result.push_str("\r\n");
                continue;
            }
        }
        result.push_str(line);
        result.push_str("\r\n");
    }
    if in_audio {
        result.push_str(&rtpmaps);
    }
    result
}

/// The same offer with RFC 2833 taken out of it: the `telephone-event`
/// payload type leaves the audio `m=` line and its `a=rtpmap`/`a=fmtp`
/// lines go with it.
///
/// A trunk that takes DTMF as SIP INFO, or takes none, is not offered
/// `telephone-event` (`docs/TENANT_TRUNKS.md` §6) — offering it and then
/// not sending it is how DTMF goes missing on a carrier that believes the
/// offer.
pub fn drop_telephone_event(sdp_body: &str) -> String {
    let Some(te_pt) = extract_telephone_event_pt(sdp_body) else {
        return sdp_body.to_string();
    };

    let mut result = String::new();
    let mut audio_seen = false;
    let mut in_audio = false;
    for line in sdp_body.lines() {
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
                        if pt.parse::<u8>() == Ok(te_pt) {
                            continue;
                        }
                        result.push(' ');
                        result.push_str(pt);
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
                if pt_str.parse::<u8>() == Ok(te_pt) {
                    continue;
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

    fn pcmu() -> SdpCodecInfo {
        SdpCodecInfo {
            payload_type: 0,
            encoding_name: "PCMU".into(),
            clock_rate: 8000,
        }
    }

    #[test]
    fn offer_with_audio_video_and_srtp_round_trips() {
        let audio_key = SrtpOffer::generate().unwrap();
        let video_key = SrtpOffer::generate().unwrap();
        let codecs = [
            SdpCodecInfo {
                payload_type: 9,
                encoding_name: "G722".into(),
                clock_rate: 8000,
            },
            pcmu(),
        ];
        let offer = construct_offer(&OfferOptions {
            server_ip: "10.0.0.5",
            origin_id: "42",
            session_name: "gateway",
            audio: AudioOfferSection {
                rtp_port: 20000,
                codecs: &codecs,
                telephone_event: Some((101, 8000)),
                srtp: Some(&audio_key.sdp),
                ptime_ms: Some(20),
            },
            video: Some(VideoOfferSection {
                rtp_port: 20002,
                payload_type: 96,
                codec: VideoCodec::H264,
                fmtp: Some(
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
                ),
                feedback: true,
                srtp: Some(&video_key.sdp),
                simulcast: None,
                transport_cc: None,
            }),
            content: None,
        })
        .unwrap();
        for needle in [
            "o=fcp 42",
            "c=IN IP4 10.0.0.5\r\n",
            "m=audio 20000 RTP/SAVP 9 0 101\r\n",
            "a=rtpmap:9 G722/8000\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n",
            "a=fmtp:101 0-16\r\n",
            "a=ptime:20\r\n",
            "m=video 20002 RTP/SAVPF 96\r\n",
            "a=rtpmap:96 H264/90000\r\n",
            "a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n",
            "a=rtcp-fb:96 nack pli\r\n",
            "a=rtcp-fb:96 goog-remb\r\n",
        ] {
            assert!(offer.contains(needle), "missing {needle:?} in {offer}");
        }
        assert_eq!(offer.matches("a=sendrecv\r\n").count(), 2);
        // Each section carries its own key, and the parsers read them back.
        let audio_crypto = parse_sdes_crypto(&offer);
        let video_crypto = parse_video_sdes_crypto(&offer);
        assert_eq!(audio_crypto.len(), 1);
        assert_eq!(video_crypto.len(), 1);
        assert_ne!(audio_crypto[0].key_params, video_crypto[0].key_params);
        assert_eq!(audio_crypto[0].suite, SrtpOffer::SUITE);
        let parsed = parse_sdp_codecs(&offer);
        assert_eq!(parsed[0].encoding_name, "G722");
        let video = parse_video_offer(&offer).unwrap();
        assert_eq!(video.port, 20002);
        assert_eq!(video.codecs[0].payload_type, 96);
        assert_eq!(video.codecs[0].codec, VideoCodec::H264);
        assert!(video.feedback && video.crypto);
        assert_eq!(remote_video_endpoint(&offer).unwrap().port(), 20002);
    }

    #[test]
    fn offer_without_srtp_or_video_is_plain_avp() {
        let codecs = [pcmu()];
        let offer = construct_offer(&OfferOptions {
            server_ip: "10.0.0.5",
            origin_id: "1",
            session_name: "call",
            audio: AudioOfferSection {
                rtp_port: 30000,
                codecs: &codecs,
                telephone_event: None,
                srtp: None,
                ptime_ms: None,
            },
            video: None,
            content: None,
        })
        .unwrap();
        assert!(offer.contains("m=audio 30000 RTP/AVP 0\r\n"), "{offer}");
        assert!(!offer.contains("m=video"), "{offer}");
        assert!(!offer.contains("a=crypto"), "{offer}");
        assert!(parse_video_offer(&offer).is_none());
        assert!(construct_offer(&OfferOptions {
            server_ip: "10.0.0.5",
            origin_id: "1",
            session_name: "call",
            audio: AudioOfferSection {
                rtp_port: 30000,
                codecs: &[],
                telephone_event: None,
                srtp: None,
                ptime_ms: None,
            },
            video: None,
            content: None,
        })
        .is_err());
    }

    #[test]
    fn srtp_offer_accepts_the_peers_key_and_refuses_a_changed_suite() {
        let ours = SrtpOffer::generate().unwrap();
        let (_, their_b64) = generate_srtp_key_material().unwrap();
        let answer = SdesCryptoLine {
            tag: 1,
            suite: SrtpOffer::SUITE.to_string(),
            key_params: format!("inline:{their_b64}"),
        };
        let ctx = ours.clone().accept(&answer).unwrap();
        assert!(ctx.is_enabled());
        let changed = SdesCryptoLine {
            suite: "AES_CM_128_HMAC_SHA1_32".to_string(),
            ..answer
        };
        assert!(ours.accept(&changed).is_err());
    }

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

        // The origin names the node, keeping the session's id and version;
        // the caller's address appears nowhere.
        assert!(rewritten.contains("o=- 123456 123456 IN IP4 10.0.0.5\r\n"));
        assert!(!rewritten.contains("192.168.1.100"), "{rewritten}");

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
    fn a_trunk_that_takes_no_rfc_2833_is_offered_none() {
        let sdp = "v=0\r\n\
                   o=- 123 1 IN IP4 192.168.1.100\r\n\
                   s=Test\r\n\
                   c=IN IP4 192.168.1.100\r\n\
                   t=0 0\r\n\
                   m=audio 10000 RTP/AVP 0 8 101\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=rtpmap:8 PCMA/8000\r\n\
                   a=rtpmap:101 telephone-event/8000\r\n\
                   a=fmtp:101 0-15\r\n\
                   a=sendrecv\r\n";

        let without = drop_telephone_event(sdp);
        assert!(
            without.contains("m=audio 10000 RTP/AVP 0 8\r\n"),
            "{without}"
        );
        assert!(!without.contains("telephone-event"), "{without}");
        assert!(!without.contains("a=fmtp:101"), "{without}");
        assert!(without.contains("a=rtpmap:0 PCMU/8000"));
        assert!(without.contains("a=rtpmap:8 PCMA/8000"));
        assert!(without.contains("a=sendrecv"), "the rest is untouched");

        // An offer that never had it is left exactly as it is.
        assert_eq!(drop_telephone_event(&without), without);
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

    /// A plain anchored offer made secure for a device the node keys:
    /// `RTP/SAVP` and our key, nothing else changed.
    #[test]
    fn with_sdes_secures_the_audio_section_only() {
        let plain = "v=0\r\nc=IN IP4 10.0.0.5\r\nt=0 0\r\nm=audio 20000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\nm=video 0 RTP/AVP 96\r\n";
        let ctx = SrtpSdpContext {
            local_key_b64: "inline:WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let secure = with_sdes(plain, &ctx);
        assert!(
            secure.contains(
                "m=audio 20000 RTP/SAVP 0 101\r\na=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:"
            ),
            "{secure}"
        );
        assert!(secure.contains("m=video 0 RTP/AVP 96"), "{secure}");
        assert_eq!(parse_sdes_crypto(&secure).len(), 1);
    }

    #[test]
    fn an_answering_key_takes_the_offers_tag_and_keeps_our_suite() {
        let offered = SdesCryptoLine {
            tag: 7,
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            key_params: "inline:WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz".into(),
        };
        let ours = SrtpOffer::answering(&offered).unwrap();
        assert_eq!(ours.sdp.tag, 7);
        assert!(ours.clone().accept(&offered).is_ok());
        let other = SdesCryptoLine {
            suite: "F8_128_HMAC_SHA1_80".into(),
            ..offered
        };
        assert!(SrtpOffer::answering(&other).is_err());
    }

    /// An encrypted caller's offer, sent on to a leg the node does not key:
    /// plain RTP, and the caller's key nowhere in it.
    #[test]
    fn an_unkeyed_anchored_leg_is_plain_and_carries_no_key() {
        let original = "v=0\r\n\
                         o=- 123 1 IN IP4 192.168.1.100\r\n\
                         s=Test\r\n\
                         c=IN IP4 192.168.1.100\r\n\
                         t=0 0\r\n\
                         m=audio 10000 RTP/SAVP 0 101\r\n\
                         a=rtpmap:0 PCMU/8000\r\n\
                         a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:WVNfX19zZW1jdGwgKCkgewkyMjA7fQp9CnVubGVz\r\n";
        let rewritten = rewrite_sdp_for_anchoring(original, "10.0.0.5", 20000, None);
        assert!(
            rewritten.contains("m=audio 20000 RTP/AVP 0 101"),
            "{rewritten}"
        );
        assert!(
            !rewritten.contains("a=crypto"),
            "the caller's key leaked: {rewritten}"
        );
        assert!(parse_sdes_crypto(&rewritten).is_empty());
    }

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

    /// Without a key of the node's, an existing crypto line is dropped: the
    /// node terminates each leg's media, so one leg's key is never the
    /// other's.
    #[test]
    fn test_rewrite_sdp_none_context_drops_crypto() {
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
        // The peer's key is not passed on, and none is injected.
        assert_eq!(rewritten.matches("a=crypto:").count(), 0, "{rewritten}");
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
                simulcast: 0,
                transport_cc: true,
            }),
            content: None,
            bfcp: None,
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
                simulcast: 0,
                transport_cc: true,
            }),
            content: None,
            bfcp: None,
        };
        let answer = construct_answer(AV_OFFER, &opts).unwrap();
        assert_eq!(sections(&answer.sdp)[1], "m=video 0 RTP/AVPF 96 97");
        assert!(answer.video.is_none());
    }

    #[test]
    fn video_crypto_is_read_from_the_video_section_only() {
        let offer = "v=0\r\n\
o=- 1 1 IN IP4 192.168.1.50\r\n\
s=-\r\n\
c=IN IP4 192.168.1.50\r\n\
t=0 0\r\n\
m=audio 4000 RTP/SAVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:audiokeyaudiokeyaudiokeyaudiokeyaudio\r\n\
m=video 4002 RTP/SAVPF 96\r\n\
a=rtpmap:96 VP8/90000\r\n\
a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:videokeyvideokeyvideokeyvideokeyvideo\r\n";
        let video = parse_video_sdes_crypto(offer);
        assert_eq!(video.len(), 1);
        assert_eq!(video[0].tag, 2);
        assert_eq!(video[0].suite, "AES_CM_128_HMAC_SHA1_32");
        assert!(video[0].key_params.starts_with("inline:videokey"));
        let audio = parse_sdes_crypto(offer);
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].tag, 1);
        // An audio-only offer has no video crypto; garbage has none.
        assert!(parse_video_sdes_crypto(&offer[..offer.find("m=video").unwrap()]).is_empty());
        assert!(parse_video_sdes_crypto("not sdp").is_empty());
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
                simulcast: 0,
                transport_cc: true,
            }),
            content: None,
            bfcp: None,
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

    /// A Cisco-shaped room system: UDP/BFCP, `c-only`, the content
    /// section labelled, and a Poly-shaped one over TCP with `actpass`.
    fn room_system_offer(bfcp_line: &str, extra: &str) -> String {
        format!(
            "v=0\r\no=- 1 1 IN IP4 10.1.1.1\r\ns=-\r\nc=IN IP4 10.1.1.1\r\nt=0 0\r\n\
             m=audio 4000 RTP/AVP 0\r\n\
             m=video 4002 RTP/AVP 96\r\na=rtpmap:96 VP8/90000\r\na=label:11\r\n\
             m=video 4004 RTP/AVP 96\r\na=rtpmap:96 VP8/90000\r\na=content:slides\r\na=label:12\r\n\
             {bfcp_line}\r\na=floorctrl:c-only\r\n{extra}"
        )
    }

    #[test]
    fn a_bfcp_section_is_answered_as_the_floor_control_server() {
        let offer = room_system_offer("m=application 5070 UDP/BFCP *", "a=bfcpver:2\r\n");
        let parsed = parse_bfcp_offer(&offer).expect("a BFCP section");
        assert_eq!(parsed.transport, BfcpTransport::Udp);
        assert_eq!(parsed.port, 5070);
        assert_eq!(parsed.remote, "10.1.1.1:5070".parse().ok());
        assert_eq!(parsed.floorctrl, vec!["c-only"]);
        assert_eq!(parsed.versions, vec![2]);
        assert!(parsed.answerable());
        let codecs = [VideoCodec::VP8];
        let answer = construct_answer(
            &offer,
            &AnswerOptions {
                server_ip: "10.0.0.5",
                audio_rtp_port: 20000,
                audio_srtp: None,
                video: Some(VideoAnswer {
                    rtp_port: 20002,
                    codecs: &codecs,
                    srtp: None,
                    simulcast: 0,
                    transport_cc: true,
                }),
                content: Some(VideoAnswer {
                    rtp_port: 20004,
                    codecs: &codecs,
                    srtp: None,
                    simulcast: 0,
                    transport_cc: true,
                }),
                bfcp: Some(BfcpAnswer {
                    port: 5060,
                    conference_id: 4321,
                    user_id: 7,
                    floor_id: 1,
                    label: "12",
                }),
            },
        )
        .unwrap();
        let m = sections(&answer.sdp);
        assert_eq!(m[3], "m=application 5060 UDP/BFCP *");
        let bfcp = &answer.sdp[answer.sdp.find("m=application").unwrap()..];
        for line in [
            "a=floorctrl:s-only",
            "a=confid:4321",
            "a=userid:7",
            "a=floorid:1 mstrm:12",
            "a=bfcpver:2",
        ] {
            assert!(bfcp.contains(&format!("{line}\r\n")), "{line} in {bfcp}");
        }
        assert!(!bfcp.contains("a=setup"), "UDP has no setup");
        let negotiated = answer.bfcp.expect("negotiated");
        assert_eq!(negotiated.transport, BfcpTransport::Udp);
        assert_eq!(
            (
                negotiated.conference_id,
                negotiated.user_id,
                negotiated.floor_id
            ),
            (4321, 7, 1)
        );
        assert_eq!(negotiated.remote, "10.1.1.1:5070".parse().ok());
        // The content section went through beside it.
        assert_eq!(
            answer
                .content
                .as_ref()
                .and_then(|c| c.label.clone())
                .as_deref(),
            Some("12")
        );

        // Poly-shaped: TCP, actpass. We are passive and the client connects.
        let offer = room_system_offer(
            "m=application 5070 TCP/BFCP *",
            "a=setup:actpass\r\na=connection:new\r\na=bfcpver:1\r\n",
        );
        let parsed = parse_bfcp_offer(&offer).unwrap();
        assert_eq!(parsed.transport, BfcpTransport::Tcp);
        assert_eq!(parsed.setup.as_deref(), Some("actpass"));
        assert!(parsed.answerable());
        let answer = construct_answer(
            &offer,
            &AnswerOptions {
                server_ip: "10.0.0.5",
                audio_rtp_port: 20000,
                audio_srtp: None,
                video: Some(VideoAnswer {
                    rtp_port: 20002,
                    codecs: &codecs,
                    srtp: None,
                    simulcast: 0,
                    transport_cc: true,
                }),
                content: Some(VideoAnswer {
                    rtp_port: 20004,
                    codecs: &codecs,
                    srtp: None,
                    simulcast: 0,
                    transport_cc: true,
                }),
                bfcp: Some(BfcpAnswer {
                    port: 5060,
                    conference_id: 1,
                    user_id: 2,
                    floor_id: 1,
                    label: "12",
                }),
            },
        )
        .unwrap();
        let bfcp = &answer.sdp[answer.sdp.find("m=application").unwrap()..];
        assert!(bfcp.starts_with("m=application 5060 TCP/BFCP *\r\n"));
        assert!(bfcp.contains("a=setup:passive\r\n"));
        assert!(bfcp.contains("a=connection:new\r\n"));
        assert!(bfcp.contains("a=bfcpver:1\r\n"));
        assert_eq!(answer.bfcp.unwrap().transport, BfcpTransport::Tcp);
    }

    #[test]
    fn a_bfcp_section_is_rejected_when_it_cannot_be_served() {
        let codecs = [VideoCodec::VP8];
        let opts = |bfcp| AnswerOptions {
            server_ip: "10.0.0.5",
            audio_rtp_port: 20000,
            audio_srtp: None,
            video: Some(VideoAnswer {
                rtp_port: 20002,
                codecs: &codecs,
                srtp: None,
                simulcast: 0,
                transport_cc: true,
            }),
            content: Some(VideoAnswer {
                rtp_port: 20004,
                codecs: &codecs,
                srtp: None,
                simulcast: 0,
                transport_cc: true,
            }),
            bfcp,
        };
        let serve = BfcpAnswer {
            port: 5060,
            conference_id: 1,
            user_id: 2,
            floor_id: 1,
            label: "12",
        };
        // Nobody listening: port 0.
        let offer = room_system_offer("m=application 5070 UDP/BFCP *", "");
        let answer = construct_answer(&offer, &opts(None)).unwrap();
        assert_eq!(sections(&answer.sdp)[3], "m=application 0 UDP/BFCP *");
        assert!(answer.bfcp.is_none());
        // The offerer will only be the server: nothing for us to be.
        let offer =
            room_system_offer("m=application 5070 UDP/BFCP *", "").replace("c-only", "s-only");
        assert!(!parse_bfcp_offer(&offer).unwrap().answerable());
        let answer = construct_answer(&offer, &opts(Some(serve.clone()))).unwrap();
        assert_eq!(sections(&answer.sdp)[3], "m=application 0 UDP/BFCP *");
        // TCP where the offerer expects us to connect.
        let offer = room_system_offer("m=application 5070 TCP/BFCP *", "a=setup:passive\r\n");
        assert!(!parse_bfcp_offer(&offer).unwrap().answerable());
        // TLS: not spoken here.
        let offer = room_system_offer("m=application 5070 UDP/TLS/BFCP *", "");
        assert!(parse_bfcp_offer(&offer).is_none());
        let answer = construct_answer(&offer, &opts(Some(serve.clone()))).unwrap();
        assert_eq!(sections(&answer.sdp)[3], "m=application 0 UDP/TLS/BFCP *");
        // A version we do not speak on that transport.
        let offer = room_system_offer("m=application 5070 UDP/BFCP *", "a=bfcpver:1\r\n");
        assert!(!parse_bfcp_offer(&offer).unwrap().answerable());
        // `c-s` offered: the offerer will be the client if we are the server.
        let offer = room_system_offer("m=application 5070 UDP/BFCP *", "").replace("c-only", "c-s");
        assert!(parse_bfcp_offer(&offer).unwrap().answerable());
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
                content: None,
            },
            AnchorKeys::default(),
        );
        let m = sections(&rewritten);
        assert_eq!(m[1], "m=video 20002 RTP/AVPF 96 97");
    }

    /// AV_OFFER's camera sending three simulcast streams (RFC 8853), the
    /// rid extension at session level.
    #[test]
    fn transport_cc_is_kept_when_offered_and_carried_and_left_out_otherwise() {
        let uri = forge_rtp::transport_cc::TRANSPORT_CC_URI;
        let offer = AV_OFFER.replace(
            "a=rtcp-fb:* ccm fir\r\n",
            &format!("a=rtcp-fb:* ccm fir\r\na=rtcp-fb:* transport-cc\r\na=extmap:7 {uri}\r\n"),
        );
        let codecs = [VideoCodec::H264];
        let answer = |carried: bool| {
            construct_answer(
                &offer,
                &AnswerOptions {
                    server_ip: "10.0.0.5",
                    audio_rtp_port: 20000,
                    audio_srtp: None,
                    video: Some(VideoAnswer {
                        rtp_port: 20002,
                        codecs: &codecs,
                        srtp: None,
                        simulcast: 0,
                        transport_cc: carried,
                    }),
                    content: None,
                    bfcp: None,
                },
            )
            .unwrap()
        };
        let kept = answer(true);
        let v = kept.video.unwrap();
        assert_eq!(v.transport_cc, Some(7));
        let pt = v.payload_type;
        assert!(
            kept.sdp.contains(&format!("a=extmap:7 {uri}\r\n")),
            "{}",
            kept.sdp
        );
        assert!(
            kept.sdp
                .contains(&format!("a=rtcp-fb:{pt} transport-cc\r\n")),
            "{}",
            kept.sdp
        );
        let left = answer(false);
        assert_eq!(left.video.unwrap().transport_cc, None);
        assert!(!left.sdp.contains("transport"), "{}", left.sdp);
        // Offered without the extension: feedback alone means nothing.
        let bare = AV_OFFER.replace(
            "a=rtcp-fb:* ccm fir\r\n",
            "a=rtcp-fb:* ccm fir\r\na=rtcp-fb:* transport-cc\r\n",
        );
        let a = construct_answer(
            &bare,
            &AnswerOptions {
                server_ip: "10.0.0.5",
                audio_rtp_port: 20000,
                audio_srtp: None,
                video: Some(VideoAnswer {
                    rtp_port: 20002,
                    codecs: &codecs,
                    srtp: None,
                    simulcast: 0,
                    transport_cc: true,
                }),
                content: None,
                bfcp: None,
            },
        )
        .unwrap();
        assert_eq!(a.video.unwrap().transport_cc, None);
    }

    fn simulcast_offer() -> String {
        AV_OFFER
            .replace(
                "t=0 0\r\n",
                "t=0 0\r\na=extmap:3 urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id\r\n",
            )
            .replace(
                "a=rtcp-fb:* ccm fir\r\n",
                "a=rtcp-fb:* ccm fir\r\na=rid:f send pt=96,97;max-height=720\r\n\
                 a=rid:h send\r\na=rid:q send\r\na=simulcast:send f;h;~q\r\n",
            )
    }

    fn simulcast_answer(simulcast: usize) -> Answer {
        let codecs = [VideoCodec::H264];
        construct_answer(
            &simulcast_offer(),
            &AnswerOptions {
                server_ip: "10.0.0.5",
                audio_rtp_port: 20000,
                audio_srtp: None,
                video: Some(VideoAnswer {
                    rtp_port: 20002,
                    codecs: &codecs,
                    srtp: None,
                    simulcast,
                    transport_cc: true,
                }),
                content: None,
                bfcp: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn a_camera_s_simulcast_is_taken_when_the_answer_asks_for_it() {
        let answer = simulcast_answer(3);
        let video = &answer.sdp[answer.sdp.find("m=video").unwrap()..];
        assert!(
            video.contains("a=extmap:3 urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id\r\n"),
            "{video}"
        );
        // Received, in the offer's order, the pause kept, and none of the
        // offer's restrictions (its pt=96 names a format we did not take).
        assert!(video.contains("a=rid:f recv\r\n"), "{video}");
        assert!(video.contains("a=rid:h recv\r\n"), "{video}");
        assert!(video.contains("a=rid:q recv\r\n"), "{video}");
        assert!(video.contains("a=simulcast:recv f;h;~q\r\n"), "{video}");
        assert!(!video.contains("max-height"), "{video}");
        let layers = answer.video.unwrap().simulcast.expect("simulcast taken");
        let rids: Vec<&str> = layers.rids.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(rids, ["f", "h", "q"]);
        assert_eq!(layers.paused, [false, false, true]);
        assert_eq!(layers.rid_extension, Some(3));
    }

    #[test]
    fn simulcast_is_capped_and_left_alone_when_not_asked_for() {
        let two = simulcast_answer(2);
        assert!(two.sdp.contains("a=simulcast:recv f;h\r\n"), "{}", two.sdp);
        assert_eq!(two.video.unwrap().simulcast.unwrap().rids.len(), 2);

        let none = simulcast_answer(0);
        assert!(!none.sdp.contains("a=rid"), "{}", none.sdp);
        assert!(!none.sdp.contains("a=simulcast"), "{}", none.sdp);
        assert!(none.video.unwrap().simulcast.is_none());
    }

    #[test]
    fn anchoring_carries_simulcast_across_the_node() {
        let rewritten = rewrite_for_anchoring(
            &simulcast_offer(),
            "10.0.0.5",
            AnchorPorts {
                audio: 20000,
                video: Some(20002),
                content: None,
            },
            AnchorKeys::default(),
        );
        for line in [
            "a=extmap:3 urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id\r\n",
            "a=rid:f send pt=96,97;max-height=720\r\n",
            "a=simulcast:send f;h;~q\r\n",
        ] {
            assert!(rewritten.contains(line), "{line} in {rewritten}");
        }
    }

    #[test]
    fn a_relayed_video_section_is_keyed_with_the_nodes_own_key() {
        let offer = AV_OFFER.replace(
            "a=rtcp-fb:* ccm fir\r\n",
            "a=rtcp-fb:* ccm fir\r\na=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:theirs\r\n",
        );
        let ours = SrtpSdpContext {
            local_key_b64: "inline:ours".into(),
            suite: "AES_CM_128_HMAC_SHA1_80".into(),
            tag: 1,
        };
        let ports = AnchorPorts {
            audio: 20000,
            video: Some(20002),
            content: None,
        };
        let keyed = rewrite_for_anchoring(
            &offer,
            "10.0.0.5",
            ports,
            AnchorKeys {
                audio: None,
                video: Some(&ours),
                content: None,
            },
        );
        assert_eq!(sections(&keyed)[1], "m=video 20002 RTP/SAVPF 96 97");
        assert!(keyed.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:ours"));
        assert!(
            !keyed.contains("inline:theirs"),
            "the other leg's key stays on its side"
        );

        // A plain leg is offered plain video, the other leg's key dropped.
        let plain = rewrite_for_anchoring(&offer, "10.0.0.5", ports, AnchorKeys::default());
        assert_eq!(sections(&plain)[1], "m=video 20002 RTP/AVPF 96 97");
        assert!(!plain.contains("a=crypto"), "{plain}");
    }

    #[test]
    fn the_callers_answer_takes_the_callees_video_on_the_nodes_port() {
        // The node's own answer to the caller turns video down; the
        // callee chose H.264 at a lower level.
        let own = construct_answer_for_a_leg(AV_OFFER, "10.0.0.5", 20000, None);
        let callee = "v=0\r\no=- 9 9 IN IP4 172.16.0.9\r\ns=-\r\nc=IN IP4 172.16.0.9\r\nt=0 0\r\n\
             m=audio 6000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n\
             m=video 6002 RTP/SAVPF 97\r\na=rtpmap:97 H264/90000\r\n\
             a=fmtp:97 profile-level-id=42e00d;packetization-mode=1\r\n\
             a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:callee\r\n\
             a=rtcp-fb:97 nack pli\r\na=sendrecv\r\n";
        let spliced = splice_relayed_video(
            &own,
            callee,
            AV_OFFER,
            "10.0.0.5",
            AnchorPorts {
                audio: 20000,
                video: Some(20002),
                content: None,
            },
            AnchorKeys::default(),
        );
        let m = sections(&spliced);
        assert_eq!(m[0], sections(&own)[0], "audio is the node's own answer");
        assert_eq!(m[1], "m=video 20002 RTP/AVPF 97");
        assert!(
            spliced.contains("profile-level-id=42e00d"),
            "the callee's parameters"
        );
        assert!(spliced.contains("a=rtcp-fb:97 nack pli"));
        assert!(
            !spliced.contains("172.16.0.9"),
            "the callee's address stays on its side"
        );
        assert!(!spliced.contains("inline:callee"));

        // A callee that turned video down leaves it down.
        let declined = callee.replace("m=video 6002", "m=video 0");
        let spliced = splice_relayed_video(
            &own,
            &declined,
            AV_OFFER,
            "10.0.0.5",
            AnchorPorts {
                audio: 20000,
                video: Some(20002),
                content: None,
            },
            AnchorKeys::default(),
        );
        assert_eq!(spliced, own);
    }

    #[test]
    fn a_holds_video_is_answered_inactive_and_nothing_else_moves() {
        let sdp = "v=0\r\no=- 1 1 IN IP4 10.0.0.5\r\ns=-\r\nc=IN IP4 10.0.0.5\r\nt=0 0\r\n\
             m=audio 20000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=recvonly\r\n\
             m=video 20002 RTP/AVPF 97\r\na=rtpmap:97 H264/90000\r\na=sendrecv\r\n\
             m=video 0 RTP/AVPF 97\r\na=content:slides\r\n";
        let held = with_video_direction(sdp, "inactive");
        let (_, sections) = split_sections(&held);
        assert!(sections[0].contains("a=recvonly"), "audio keeps its own");
        assert!(sections[1].ends_with("a=inactive\r\n"), "{held}");
        assert!(!sections[1].contains("sendrecv"));
        assert!(
            !sections[2].contains("a=inactive"),
            "a turned-down section stays as it was"
        );
        // A section with no direction is given one.
        let bare = sdp.replace("a=sendrecv\r\n", "");
        assert!(
            split_sections(&with_video_direction(&bare, "inactive")).1[1]
                .ends_with("a=inactive\r\n")
        );
    }

    #[test]
    fn an_attendee_s_video_is_answered_sending_only() {
        let answer = "v=0\r\no=- 1 1 IN IP4 10.0.0.5\r\ns=-\r\nc=IN IP4 10.0.0.5\r\nt=0 0\r\n\
             m=audio 20000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\n\
             m=video 20002 RTP/AVPF 97\r\na=rtpmap:97 VP8/90000\r\na=sendrecv\r\n\
             m=video 20004 RTP/AVPF 97\r\na=content:slides\r\na=recvonly\r\n\
             m=video 0 RTP/AVPF 97\r\n";
        let only = with_video_sending_only(answer);
        let (_, sections) = split_sections(&only);
        assert!(sections[0].contains("a=sendrecv"), "audio keeps its own");
        assert!(sections[1].ends_with("a=sendonly\r\n"), "{only}");
        assert!(!sections[1].contains("sendrecv"));
        // The offer only sent its screen: nothing left to answer with.
        assert!(sections[2].ends_with("a=inactive\r\n"), "{only}");
        assert!(!sections[3].contains("a="), "a turned-down section stays");
        // No direction is sendrecv, and so sendonly.
        let bare = answer.replacen("a=sendrecv\r\nm=video", "m=video", 1);
        let bare = bare.replacen("a=sendrecv\r\n", "", 1);
        assert!(split_sections(&with_video_sending_only(&bare)).1[1].ends_with("a=sendonly\r\n"));
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

#[cfg(test)]
mod ensure_audio_codecs_tests {
    use super::*;

    const OPUS_ONLY: &str =
        "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
        m=audio 4000 RTP/AVP 96 101\r\na=rtpmap:96 opus/48000/2\r\n\
        a=rtpmap:101 telephone-event/8000\r\na=sendrecv\r\n\
        m=video 4002 RTP/AVP 97\r\na=rtpmap:97 VP8/90000\r\n";

    #[test]
    fn a_codec_the_offer_lacks_is_added_to_its_audio_section() {
        let sdp = ensure_audio_codecs(OPUS_ONLY, &["opus", "PCMU", "PCMA"]);
        let names: Vec<String> = parse_sdp_codecs(&sdp)
            .into_iter()
            .map(|c| c.encoding_name)
            .collect();
        assert!(names.iter().any(|n| n == "PCMU"), "{sdp}");
        assert!(names.iter().any(|n| n == "PCMA"), "{sdp}");
        assert!(sdp.contains("m=audio 4000 RTP/AVP 96 101 0 8\r\n"), "{sdp}");
        // The rtpmaps stay in the audio section, before the video one.
        let audio_end = sdp.find("m=video").unwrap();
        assert!(
            sdp.find("a=rtpmap:8 PCMA/8000").unwrap() < audio_end,
            "{sdp}"
        );
        assert!(sdp.contains("m=video 4002 RTP/AVP 97\r\n"), "{sdp}");
    }

    #[test]
    fn filtered_after_adding_the_offer_is_what_was_asked_for() {
        let sdp = filter_sdp_codecs(
            &ensure_audio_codecs(OPUS_ONLY, &["opus", "PCMU"]),
            &["opus", "PCMU"],
        );
        assert!(sdp.contains("m=audio 4000 RTP/AVP 96 101 0\r\n"), "{sdp}");
    }

    #[test]
    fn opus_takes_a_free_dynamic_payload_type_and_nothing_is_added_twice() {
        let pcmu = "v=0\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0 96\r\n\
            a=rtpmap:0 PCMU/8000\r\na=rtpmap:96 telephone-event/8000\r\n";
        let sdp = ensure_audio_codecs(pcmu, &["PCMU", "opus", "AMR"]);
        assert!(sdp.contains("m=audio 4000 RTP/AVP 0 96 97\r\n"), "{sdp}");
        assert!(sdp.contains("a=rtpmap:97 opus/48000/2"), "{sdp}");
        assert_eq!(ensure_audio_codecs(pcmu, &["PCMU"]), pcmu);
    }
}
