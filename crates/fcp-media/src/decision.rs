//! Media anchoring decision logic
//!
//! Determines when RTP should be anchored on the server vs. pass-through mode.

use crate::codec;

/// Decision on whether to anchor media on the server
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaAnchoringDecision {
    /// Pass-through mode: RTP flows directly between endpoints
    PassThrough { reason: String },
    /// Anchored mode: RTP flows through the server
    Anchored {
        reason: String,
        requires_transcoding: bool,
    },
}

/// Context for making media anchoring decisions
#[derive(Debug, Clone)]
pub struct MediaDecisionContext {
    /// Whether media anchoring is globally enabled in config
    pub media_enabled: bool,

    /// Whether transcoding is enabled in config
    pub transcoding_enabled: bool,

    /// Whether recording is enabled in config
    pub recording_enabled: bool,

    /// Whether DTMF detection is enabled in config
    pub dtmf_enabled: bool,

    /// Codecs from A-leg (caller) SDP
    pub a_leg_codecs: Vec<String>,

    /// Codecs from B-leg (callee) - may be empty if not yet known
    pub b_leg_codecs: Option<Vec<String>>,

    /// Whether recording is requested for this call
    pub recording_requested: bool,

    /// Whether DTMF injection/detection is needed for this call
    pub dtmf_requested: bool,

    /// Whether this is a conference call
    pub conference_call: bool,
}

impl MediaDecisionContext {
    /// Create a new decision context with defaults
    pub fn new(media_enabled: bool) -> Self {
        Self {
            media_enabled,
            transcoding_enabled: false,
            recording_enabled: false,
            dtmf_enabled: true,
            a_leg_codecs: Vec::new(),
            b_leg_codecs: None,
            recording_requested: false,
            dtmf_requested: false,
            conference_call: false,
        }
    }

    /// Update with codecs from A-leg (caller)
    pub fn with_a_leg_codecs(mut self, codecs: Vec<String>) -> Self {
        self.a_leg_codecs = codecs;
        self
    }

    /// Update with codecs from B-leg (callee)
    pub fn with_b_leg_codecs(mut self, codecs: Vec<String>) -> Self {
        self.b_leg_codecs = Some(codecs);
        self
    }

    /// Enable recording for this call
    pub fn with_recording(mut self) -> Self {
        self.recording_requested = true;
        self
    }

    /// Enable DTMF handling for this call
    pub fn with_dtmf(mut self) -> Self {
        self.dtmf_requested = true;
        self
    }

    /// Mark as conference call
    pub fn with_conference(mut self) -> Self {
        self.conference_call = true;
        self
    }
}

/// Determine whether to anchor media for a call
pub fn should_anchor_media(ctx: &MediaDecisionContext) -> MediaAnchoringDecision {
    // If media is globally disabled, always pass through
    if !ctx.media_enabled {
        return MediaAnchoringDecision::PassThrough {
            reason: "Media anchoring disabled in configuration".to_string(),
        };
    }

    // Conference calls always need anchoring
    if ctx.conference_call {
        return MediaAnchoringDecision::Anchored {
            reason: "Conference call requires media mixing".to_string(),
            requires_transcoding: false,
        };
    }

    // Recording requires anchoring
    if ctx.recording_requested && ctx.recording_enabled {
        return MediaAnchoringDecision::Anchored {
            reason: "Call recording enabled".to_string(),
            requires_transcoding: false,
        };
    }

    // DTMF detection/injection requires anchoring
    if ctx.dtmf_requested && ctx.dtmf_enabled {
        return MediaAnchoringDecision::Anchored {
            reason: "DTMF detection/injection requested".to_string(),
            requires_transcoding: false,
        };
    }

    // Check if transcoding is needed (different codecs on each leg)
    if let Some(ref b_leg_codecs) = ctx.b_leg_codecs {
        if ctx.transcoding_enabled && !ctx.a_leg_codecs.is_empty() && !b_leg_codecs.is_empty() {
            // Check if there's a common codec
            let has_common_codec = codec::find_common_codec(&ctx.a_leg_codecs, b_leg_codecs);

            if has_common_codec.is_none() {
                return MediaAnchoringDecision::Anchored {
                    reason: format!(
                        "Codec transcoding required (A-leg: {:?}, B-leg: {:?})",
                        ctx.a_leg_codecs, b_leg_codecs
                    ),
                    requires_transcoding: true,
                };
            }
        }
    }

    // Default to pass-through mode
    MediaAnchoringDecision::PassThrough {
        reason: "No media processing required, using direct RTP".to_string(),
    }
}

/// Find a common codec between two codec lists
fn find_common_codec<'a>(a_leg: &'a [String], b_leg: &'a [String]) -> Option<&'a String> {
    // Normalize codec names for comparison
    for a_codec in a_leg {
        let a_canonical = codec::canonical_codec_name(a_codec);
        for b_codec in b_leg {
            let b_canonical = codec::canonical_codec_name(b_codec);
            if a_canonical == b_canonical {
                return Some(a_codec);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_through_when_disabled() {
        let ctx = MediaDecisionContext::new(false);
        let decision = should_anchor_media(&ctx);

        assert_eq!(
            decision,
            MediaAnchoringDecision::PassThrough {
                reason: "Media anchoring disabled in configuration".to_string()
            }
        );
    }

    #[test]
    fn test_anchor_for_recording() {
        let ctx = MediaDecisionContext {
            media_enabled: true,
            recording_enabled: true,
            recording_requested: true,
            ..MediaDecisionContext::new(true)
        };
        let decision = should_anchor_media(&ctx);

        match decision {
            MediaAnchoringDecision::Anchored { reason, .. } => {
                assert!(reason.contains("recording"));
            }
            _ => panic!("Expected Anchored decision"),
        }
    }

    #[test]
    fn test_anchor_for_dtmf() {
        let ctx = MediaDecisionContext {
            media_enabled: true,
            dtmf_enabled: true,
            dtmf_requested: true,
            ..MediaDecisionContext::new(true)
        };
        let decision = should_anchor_media(&ctx);

        match decision {
            MediaAnchoringDecision::Anchored { reason, .. } => {
                assert!(reason.contains("DTMF"));
            }
            _ => panic!("Expected Anchored decision"),
        }
    }

    #[test]
    fn test_anchor_for_conference() {
        let ctx = MediaDecisionContext {
            media_enabled: true,
            conference_call: true,
            ..MediaDecisionContext::new(true)
        };
        let decision = should_anchor_media(&ctx);

        match decision {
            MediaAnchoringDecision::Anchored { reason, .. } => {
                assert!(reason.contains("Conference"));
            }
            _ => panic!("Expected Anchored decision"),
        }
    }

    #[test]
    fn test_anchor_for_transcoding() {
        let ctx = MediaDecisionContext {
            media_enabled: true,
            transcoding_enabled: true,
            a_leg_codecs: vec!["PCMU".to_string()],
            b_leg_codecs: Some(vec!["opus".to_string()]),
            ..MediaDecisionContext::new(true)
        };
        let decision = should_anchor_media(&ctx);

        match decision {
            MediaAnchoringDecision::Anchored {
                reason,
                requires_transcoding,
            } => {
                assert!(reason.contains("transcoding"));
                assert!(requires_transcoding);
            }
            _ => panic!("Expected Anchored decision"),
        }
    }

    #[test]
    fn test_pass_through_with_common_codec() {
        let ctx = MediaDecisionContext {
            media_enabled: true,
            transcoding_enabled: true,
            a_leg_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            b_leg_codecs: Some(vec!["PCMA".to_string(), "opus".to_string()]),
            ..MediaDecisionContext::new(true)
        };
        let decision = should_anchor_media(&ctx);

        match decision {
            MediaAnchoringDecision::PassThrough { reason } => {
                assert!(reason.contains("No media processing required"));
            }
            _ => panic!("Expected PassThrough decision"),
        }
    }

    #[test]
    fn test_find_common_codec() {
        let a_leg = vec!["PCMU".to_string(), "PCMA".to_string()];
        let b_leg = vec!["PCMA".to_string(), "opus".to_string()];

        let common = find_common_codec(&a_leg, &b_leg);
        assert_eq!(common, Some(&"PCMA".to_string()));
    }

    #[test]
    fn test_find_common_codec_case_insensitive() {
        let a_leg = vec!["pcmu".to_string()];
        let b_leg = vec!["PCMU".to_string()];

        let common = find_common_codec(&a_leg, &b_leg);
        assert_eq!(common, Some(&"pcmu".to_string()));
    }

    #[test]
    fn test_no_common_codec() {
        let a_leg = vec!["PCMU".to_string()];
        let b_leg = vec!["opus".to_string()];

        let common = find_common_codec(&a_leg, &b_leg);
        assert_eq!(common, None);
    }
}
