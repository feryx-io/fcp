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

/// Video codec definitions (placeholder for Phase 2+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VideoCodec {
    /// H.264/AVC
    H264,

    /// VP8
    Vp8,

    /// VP9
    Vp9,

    /// H.265/HEVC
    H265,
}

impl VideoCodec {
    /// Get the RTP payload type for this codec.
    pub fn payload_type(self) -> u8 {
        match self {
            VideoCodec::H264 => 99,
            VideoCodec::Vp8 => 100,
            VideoCodec::Vp9 => 101,
            VideoCodec::H265 => 102,
        }
    }

    /// Get the codec name as it appears in SDP.
    pub fn sdp_name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H264",
            VideoCodec::Vp8 => "VP8",
            VideoCodec::Vp9 => "VP9",
            VideoCodec::H265 => "H265",
        }
    }
}

/// Unified codec type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Codec {
    Audio(AudioCodec),
    Video(VideoCodec),
}

impl Codec {
    /// Get the RTP payload type.
    pub fn payload_type(self) -> u8 {
        match self {
            Codec::Audio(c) => c.payload_type(),
            Codec::Video(c) => c.payload_type(),
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
            // Default video preference: VP8 > H.264
            video: vec![VideoCodec::Vp8, VideoCodec::H264],
        }
    }
}
