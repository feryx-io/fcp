//! Media anchoring policy configuration for B2BUA.
//!
//! Controls whether the B2BUA inserts itself into the media path (RTP/SRTP)
//! or allows media to flow directly between endpoints.
//!
//! This is *policy-level* configuration that is evaluated before the runtime
//! media decision in `fcp_media::MediaDecisionContext::should_anchor_media()`.
//! When the mode is `Always` or `Never`, the policy short-circuits the runtime
//! check. When `Conditional`, the runtime decision logic is used.
//!
//! - **SBC**: Always anchor (NAT traversal, lawful intercept, orecording)
//! - **PBX**: Conditional (anchor when needed for features)
//! - **Session Manager**: Never (routing only, no media handling)

use serde::{Deserialize, Serialize};

/// Policy for media path anchoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MediaAnchoringMode {
    /// Never anchor media — always allow direct media between endpoints.
    Never,

    /// Always anchor media — B2BUA is always in the media path.
    Always,

    /// Anchor media only when specific conditions are met (NAT, recording, etc.).
    #[default]
    Conditional,
}

/// Policy-level configuration for media anchoring.
///
/// The `anchor_for_*` flags are only consulted when `mode` is `Conditional`.
/// When `Always`, media is always anchored regardless of flags.
/// When `Never`, media is never anchored regardless of flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaAnchoringConfig {
    /// Overall anchoring mode.
    pub mode: MediaAnchoringMode,

    /// Anchor media when NAT traversal is detected.
    pub anchor_for_nat: bool,

    /// Anchor media when call recording is active.
    pub anchor_for_recording: bool,

    /// Anchor media when DTMF interception is required (e.g., IVR).
    pub anchor_for_dtmf: bool,

    /// Anchor media when transcoding between incompatible codecs is needed.
    pub anchor_for_transcoding: bool,

    /// Rewrite SDP connection (c=) line to point to the media proxy.
    pub rewrite_sdp_connection: bool,
}

impl Default for MediaAnchoringConfig {
    fn default() -> Self {
        Self {
            mode: MediaAnchoringMode::Conditional,
            anchor_for_nat: true,
            anchor_for_recording: true,
            anchor_for_dtmf: true,
            anchor_for_transcoding: true,
            rewrite_sdp_connection: true,
        }
    }
}

impl MediaAnchoringConfig {
    /// SBC defaults: always anchor media.
    pub fn sbc_defaults() -> Self {
        Self {
            mode: MediaAnchoringMode::Always,
            anchor_for_nat: true,
            anchor_for_recording: true,
            anchor_for_dtmf: true,
            anchor_for_transcoding: true,
            rewrite_sdp_connection: true,
        }
    }

    /// PBX defaults: conditional anchoring (delegates to runtime decision).
    pub fn pbx_defaults() -> Self {
        Self::default()
    }

    /// Session Manager defaults: never anchor (routing only).
    pub fn session_manager_defaults() -> Self {
        Self {
            mode: MediaAnchoringMode::Never,
            anchor_for_nat: false,
            anchor_for_recording: false,
            anchor_for_dtmf: false,
            anchor_for_transcoding: false,
            rewrite_sdp_connection: false,
        }
    }

    /// Returns `true` if media must always be anchored regardless of conditions.
    pub fn force_anchor(&self) -> bool {
        self.mode == MediaAnchoringMode::Always
    }

    /// Returns `true` if media must never be anchored regardless of conditions.
    pub fn force_passthrough(&self) -> bool {
        self.mode == MediaAnchoringMode::Never
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let cfg = MediaAnchoringConfig::default();
        assert_eq!(cfg.mode, MediaAnchoringMode::Conditional);
        assert!(cfg.anchor_for_nat);
        assert!(cfg.anchor_for_recording);
        assert!(cfg.anchor_for_dtmf);
        assert!(cfg.anchor_for_transcoding);
        assert!(cfg.rewrite_sdp_connection);
        assert!(!cfg.force_anchor());
        assert!(!cfg.force_passthrough());
    }

    #[test]
    fn test_sbc_defaults() {
        let cfg = MediaAnchoringConfig::sbc_defaults();
        assert_eq!(cfg.mode, MediaAnchoringMode::Always);
        assert!(cfg.force_anchor());
        assert!(!cfg.force_passthrough());
    }

    #[test]
    fn test_pbx_defaults() {
        let cfg = MediaAnchoringConfig::pbx_defaults();
        assert_eq!(cfg.mode, MediaAnchoringMode::Conditional);
        assert!(!cfg.force_anchor());
        assert!(!cfg.force_passthrough());
    }

    #[test]
    fn test_session_manager_defaults() {
        let cfg = MediaAnchoringConfig::session_manager_defaults();
        assert_eq!(cfg.mode, MediaAnchoringMode::Never);
        assert!(!cfg.force_anchor());
        assert!(cfg.force_passthrough());
        assert!(!cfg.anchor_for_nat);
        assert!(!cfg.anchor_for_recording);
        assert!(!cfg.anchor_for_dtmf);
        assert!(!cfg.anchor_for_transcoding);
        assert!(!cfg.rewrite_sdp_connection);
    }

    #[test]
    fn test_force_anchor_only_for_always() {
        assert!(MediaAnchoringConfig {
            mode: MediaAnchoringMode::Always,
            ..Default::default()
        }
        .force_anchor());

        assert!(!MediaAnchoringConfig {
            mode: MediaAnchoringMode::Conditional,
            ..Default::default()
        }
        .force_anchor());

        assert!(!MediaAnchoringConfig {
            mode: MediaAnchoringMode::Never,
            ..Default::default()
        }
        .force_anchor());
    }

    #[test]
    fn test_force_passthrough_only_for_never() {
        assert!(MediaAnchoringConfig {
            mode: MediaAnchoringMode::Never,
            ..Default::default()
        }
        .force_passthrough());

        assert!(!MediaAnchoringConfig {
            mode: MediaAnchoringMode::Conditional,
            ..Default::default()
        }
        .force_passthrough());

        assert!(!MediaAnchoringConfig {
            mode: MediaAnchoringMode::Always,
            ..Default::default()
        }
        .force_passthrough());
    }

    #[test]
    fn test_serde_roundtrip() {
        let cfg = MediaAnchoringConfig::sbc_defaults();
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: MediaAnchoringConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cfg, deserialized);
    }
}
