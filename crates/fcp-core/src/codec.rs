use serde::{Deserialize, Serialize};

/// Audio codec definitions for media negotiation.
///
/// The initial MVP focuses on common codecs; more will be added in later sprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioCodec {
    /// G.711 μ-law (PCMU) - 8kHz, 64kbps
    Pcmu,

    /// G.711 A-law (PCMA) - 8kHz, 64kbps
    Pcma,

    /// Opus - Variable bitrate, wideband/fullband
    Opus,

    /// G.722 - Wideband, 16kHz
    G722,

    /// G.729 - 8kHz, 8kbps
    G729,

    /// iLBC - Internet Low Bitrate Codec
    Ilbc,

    /// GSM - 8kHz, 13kbps
    Gsm,

    /// Speex - Variable bitrate
    Speex,
}

impl AudioCodec {
    /// Get the RTP payload type for this codec.
    pub fn payload_type(self) -> u8 {
        match self {
            AudioCodec::Pcmu => 0,
            AudioCodec::Pcma => 8,
            AudioCodec::G722 => 9,
            AudioCodec::Gsm => 3,
            AudioCodec::G729 => 18,
            AudioCodec::Opus => 96,  // Dynamic payload type
            AudioCodec::Ilbc => 97,  // Dynamic payload type
            AudioCodec::Speex => 98, // Dynamic payload type
        }
    }

    /// Get the codec name as it appears in SDP.
    pub fn sdp_name(self) -> &'static str {
        match self {
            AudioCodec::Pcmu => "PCMU",
            AudioCodec::Pcma => "PCMA",
            AudioCodec::Opus => "opus",
            AudioCodec::G722 => "G722",
            AudioCodec::G729 => "G729",
            AudioCodec::Ilbc => "iLBC",
            AudioCodec::Gsm => "GSM",
            AudioCodec::Speex => "speex",
        }
    }

    /// Get the sampling rate in Hz.
    pub fn sample_rate(self) -> u32 {
        match self {
            AudioCodec::Pcmu | AudioCodec::Pcma | AudioCodec::G729 | AudioCodec::Gsm => 8000,
            AudioCodec::G722 => 16000,
            AudioCodec::Opus => 48000,
            AudioCodec::Ilbc => 8000,
            AudioCodec::Speex => 8000,
        }
    }

    /// Check if this codec is wideband or better.
    pub fn is_wideband(self) -> bool {
        matches!(self, AudioCodec::G722 | AudioCodec::Opus)
    }
}

/// Video codecs FCP negotiates (the five forge-media carries; see
/// `docs/VIDEO_CONFERENCING.md` §6). Every one uses the 90 kHz RTP clock
/// and a dynamic payload type that comes from the offer, so there is no
/// fixed payload type here: the old fixed numbers put VP9 on 101, the
/// telephone-event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum VideoCodec {
    /// H.264 / AVC (RFC 6184)
    H264,
    /// H.265 / HEVC (RFC 7798)
    H265,
    /// VP8 (RFC 7741)
    Vp8,
    /// VP9 (RFC 9628)
    Vp9,
    /// AV1 (AOM RTP payload specification)
    Av1,
}

impl VideoCodec {
    /// Every codec, in the order the platform lists them.
    pub const ALL: [VideoCodec; 5] = [
        VideoCodec::H264,
        VideoCodec::H265,
        VideoCodec::Vp8,
        VideoCodec::Vp9,
        VideoCodec::Av1,
    ];

    /// RTP clock rate shared by every video codec.
    pub const CLOCK_RATE: u32 = 90_000;

    /// The codec name as it appears in SDP `a=rtpmap`.
    pub fn sdp_name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H264",
            VideoCodec::H265 => "H265",
            VideoCodec::Vp8 => "VP8",
            VideoCodec::Vp9 => "VP9",
            VideoCodec::Av1 => "AV1",
        }
    }

    /// Parse an SDP encoding name (case-insensitive; `HEVC` is accepted
    /// for H.265).
    pub fn from_sdp_name(name: &str) -> Option<Self> {
        let n = name.trim();
        VideoCodec::ALL
            .iter()
            .copied()
            .find(|c| c.sdp_name().eq_ignore_ascii_case(n))
            .or_else(|| n.eq_ignore_ascii_case("HEVC").then_some(VideoCodec::H265))
    }
}

impl std::fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sdp_name())
    }
}

impl std::str::FromStr for VideoCodec {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        VideoCodec::from_sdp_name(s).ok_or_else(|| format!("unknown video codec: {s}"))
    }
}

/// Unified codec type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Codec {
    Audio(AudioCodec),
    Video(VideoCodec),
}

impl Codec {
    /// The RTP payload type, when the codec has a fixed one. Video codecs
    /// use dynamic types negotiated per offer, so they have none.
    pub fn payload_type(self) -> Option<u8> {
        match self {
            Codec::Audio(c) => Some(c.payload_type()),
            Codec::Video(_) => None,
        }
    }

    /// Get the codec name for SDP.
    pub fn sdp_name(self) -> &'static str {
        match self {
            Codec::Audio(c) => c.sdp_name(),
            Codec::Video(c) => c.sdp_name(),
        }
    }
}

/// Codec preference list for media negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecPreferences {
    /// Preferred audio codecs in priority order.
    audio: Vec<AudioCodec>,

    /// Preferred video codecs in priority order.
    video: Vec<VideoCodec>,
}

impl CodecPreferences {
    /// Create codec preferences with explicit lists.
    pub fn new(audio: Vec<AudioCodec>, video: Vec<VideoCodec>) -> Self {
        Self { audio, video }
    }

    /// Preferred audio codecs in priority order.
    pub fn audio(&self) -> &[AudioCodec] {
        &self.audio
    }

    /// Preferred video codecs in priority order.
    pub fn video(&self) -> &[VideoCodec] {
        &self.video
    }

    /// Replace the audio preference list.
    pub fn set_audio(&mut self, audio: Vec<AudioCodec>) {
        self.audio = audio;
    }

    /// Replace the video preference list.
    pub fn set_video(&mut self, video: Vec<VideoCodec>) {
        self.video = video;
    }
}

impl Default for CodecPreferences {
    fn default() -> Self {
        Self {
            // Default audio preference: Opus > G.722 > PCMU > PCMA
            audio: vec![
                AudioCodec::Opus,
                AudioCodec::G722,
                AudioCodec::Pcmu,
                AudioCodec::Pcma,
            ],
            // Default video preference: H.264 (what SIP phones carry) > VP8
            video: vec![VideoCodec::H264, VideoCodec::Vp8],
        }
    }
}
