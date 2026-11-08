//! FCP Media Integration Layer
//!
//! This crate bridges SIP signaling (siphon-rs) with media handling (forge-media).
//! It supports two deployment models:
//!
//! - **Embedded Mode** (default): forge-media runs in-process for single-box deployments
//! - **Remote Mode** (future): forge-media runs as separate service for distributed scaling
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                    MediaManager                          │
//! │              (deployment-agnostic)                       │
//! └─────────────────┬───────────────────────────────────────┘
//!                   │
//!                   ▼
//!         ┌─────────────────────┐
//!         │  MediaBackend trait │
//!         └─────────────────────┘
//!                   │
//!         ┌─────────┴──────────┐
//!         ▼                    ▼
//!   ┌──────────────┐    ┌──────────────┐
//!   │  Embedded    │    │   Remote     │
//!   │  (Phase 1)   │    │  (Future)    │
//!   └──────────────┘    └──────────────┘
//! ```
//!
//! ## Usage
//!
//! ### Embedded Mode (Single-Box PBX)
//!
//! ```rust,no_run
//! use fcp_media::{MediaManager, MediaConfig, BackendType};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = MediaConfig {
//!         backend: BackendType::Embedded,
//!         ..Default::default()
//!     };
//!
//!     let media_manager = MediaManager::new(config).await?;
//!     // Use media_manager...
//!     Ok(())
//! }
//! ```

pub mod adapters;
pub mod backend;
pub mod codec;
pub mod config;
pub mod decision;
pub mod dtmf_collector;
pub mod error;
pub mod manager;
pub mod ports;
pub mod sdp;
pub mod session;
pub mod stt;
pub mod tts;
pub mod video_relay;

// Re-exports
pub use backend::{
    AudioTarget, EmbeddedBackend, LegDigit, MediaBackend, MediaLeg, RtpLegInfo, RtpSessionInfo,
    Supervision,
};
pub use config::{BackendType, MediaConfig};
pub use decision::{should_anchor_media, MediaAnchoringDecision, MediaDecisionContext};
pub use dtmf_collector::{CollectionResult, DtmfCollector};
pub use error::{Error, Result};
pub use manager::{MediaManager, MediaManagerStats};
pub use ports::bound_port_pair;
pub use sdp::{
    construct_answer, construct_answer_for_a_leg, construct_offer, decode_sdes_key_params,
    extract_codecs_from_sdp, extract_telephone_event_pt, filter_sdp_codecs,
    generate_srtp_key_material, parse_bfcp_offer, parse_content_sdes_crypto, parse_sdes_crypto,
    parse_sdp_codecs, parse_video_offer, parse_video_sdes_crypto, parse_video_sections,
    remote_audio_endpoint, remote_content_endpoint, remote_video_endpoint, rewrite_for_anchoring,
    rewrite_sdp_for_anchoring, sdes_suite_to_profile, srtp_context_of, video_codec_from_forge,
    video_codec_to_forge, with_sdes, with_video_direction, with_video_sdes, AnchorKeys,
    AnchorPorts, Answer, AnswerOptions, AudioOfferSection, BfcpAnswer, BfcpOffer, BfcpTransport,
    NegotiatedBfcp, NegotiatedSections, NegotiatedVideo, OfferOptions, SdesCryptoLine,
    SdpCodecInfo, SrtpOffer, SrtpSdpContext, VideoAnswer, VideoOffer, VideoOfferCodec,
    VideoOfferSection, VideoSections, VideoStream, CONTENT_LABEL,
};
pub use session::{
    DtmfMode, MediaSession, MediaSessionConfig, MediaSessionState, MediaSessionStats,
};
pub use stt::{
    SttConfig, SttOptions, SttProvider, SttProviderConfig, SttService, SttStream, Transcript,
};
pub use tts::{TtsConfig, TtsProvider, TtsProviderConfig, TtsService, VoiceInfo};
pub use video_relay::{VideoRelay, VideoRelayStats};
