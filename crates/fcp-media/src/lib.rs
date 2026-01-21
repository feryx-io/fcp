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
pub use backend::{AudioTarget, EmbeddedBackend, MediaBackend};
pub use config::{BackendType, MediaConfig};
pub use decision::{should_anchor_media, MediaAnchoringDecision, MediaDecisionContext};
pub use error::{Error, Result};
pub use manager::{MediaManager, MediaManagerStats};
pub use sdp::{extract_codecs_from_sdp, rewrite_sdp_for_anchoring};
pub use session::{
    DtmfMode, MediaSession, MediaSessionConfig, MediaSessionState, MediaSessionStats,
};
pub use tts::{TtsConfig, TtsProvider, TtsProviderConfig, TtsService, VoiceInfo};
pub use dtmf_collector::{CollectionResult, DtmfCollector};

#[cfg(feature = "remote")]
pub use backend::RemoteBackend;
