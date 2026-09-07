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
pub mod sdp;
pub mod session;
pub mod tts;

// Re-exports
pub use backend::{AudioTarget, EmbeddedBackend, MediaBackend, RtpLegInfo, RtpSessionInfo};
pub use config::{BackendType, MediaConfig};
pub use decision::{should_anchor_media, MediaAnchoringDecision, MediaDecisionContext};
pub use dtmf_collector::{CollectionResult, DtmfCollector};
pub use error::{Error, Result};
pub use manager::{MediaManager, MediaManagerStats};
pub use sdp::{
    construct_answer, construct_answer_for_a_leg, construct_offer, decode_sdes_key_params,
    extract_codecs_from_sdp, extract_telephone_event_pt, filter_sdp_codecs,
    generate_srtp_key_material, parse_sdes_crypto, parse_sdp_codecs, parse_video_offer,
    parse_video_sdes_crypto, remote_audio_endpoint, remote_video_endpoint, rewrite_for_anchoring,
    rewrite_sdp_for_anchoring, sdes_suite_to_profile, video_codec_from_forge, video_codec_to_forge,
    AnchorPorts, Answer, AnswerOptions, AudioOfferSection, NegotiatedVideo, OfferOptions,
    SdesCryptoLine, SdpCodecInfo, SrtpOffer, SrtpSdpContext, VideoAnswer, VideoOffer,
    VideoOfferCodec, VideoOfferSection,
};
pub use session::{
    DtmfMode, MediaSession, MediaSessionConfig, MediaSessionState, MediaSessionStats,
};
pub use tts::{TtsConfig, TtsProvider, TtsProviderConfig, TtsService, VoiceInfo};

#[cfg(feature = "remote")]
pub use backend::RemoteBackend;
