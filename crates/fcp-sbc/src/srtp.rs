//! SRTP policy enforcement for the SBC.

use fcp_sbc_core::trunk::{SrtpMode, TrunkSrtpConfig};
use fcp_media::sdp::parse_sdes_crypto;

/// Result of SRTP policy enforcement
#[derive(Debug)]
#[allow(dead_code)]
pub enum SrtpPolicyResult {
    /// Allow the call (SRTP not required or crypto offered)
    Allow {
        /// Whether to enable SRTP for this leg
        enable_srtp: bool,
    },
    /// Reject the call (SRTP required but no crypto offered)
    Reject {
        /// Human-readable reason for rejection
        reason: String,
    },
}

/// Enforce SRTP policy for an incoming SDP based on trunk configuration.
#[allow(dead_code)]
pub fn enforce_srtp_policy(
    trunk_srtp: &TrunkSrtpConfig,
    sdp_body: &str,
) -> SrtpPolicyResult {
    let crypto_lines = parse_sdes_crypto(sdp_body);
    let has_crypto = !crypto_lines.is_empty();

    match trunk_srtp.mode {
        SrtpMode::Disabled => SrtpPolicyResult::Allow { enable_srtp: false },
        SrtpMode::Preferred => SrtpPolicyResult::Allow { enable_srtp: has_crypto },
        SrtpMode::Required => {
            if has_crypto {
                SrtpPolicyResult::Allow { enable_srtp: true }
            } else {
                SrtpPolicyResult::Reject {
                    reason: "SRTP required but no crypto offered".into(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sdp_with_crypto() -> String {
        "v=0\r\n\
         o=- 123 1 IN IP4 10.0.0.1\r\n\
         s=Test\r\n\
         c=IN IP4 10.0.0.1\r\n\
         t=0 0\r\n\
         m=audio 5000 RTP/SAVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:PS1uQCVeeCFCanVmcjkpPywjNWhcYD0mXXtxaVBR\r\n"
            .to_string()
    }

    fn make_sdp_without_crypto() -> String {
        "v=0\r\n\
         o=- 123 1 IN IP4 10.0.0.1\r\n\
         s=Test\r\n\
         c=IN IP4 10.0.0.1\r\n\
         t=0 0\r\n\
         m=audio 5000 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n"
            .to_string()
    }

    #[test]
    fn test_enforce_srtp_disabled() {
        let config = TrunkSrtpConfig {
            mode: SrtpMode::Disabled,
            allowed_suites: Vec::new(),
        };
        let result = enforce_srtp_policy(&config, &make_sdp_with_crypto());
        match result {
            SrtpPolicyResult::Allow { enable_srtp } => assert!(!enable_srtp),
            _ => panic!("Expected Allow"),
        }
    }

    #[test]
    fn test_enforce_srtp_preferred_with_crypto() {
        let config = TrunkSrtpConfig {
            mode: SrtpMode::Preferred,
            allowed_suites: Vec::new(),
        };
        let result = enforce_srtp_policy(&config, &make_sdp_with_crypto());
        match result {
            SrtpPolicyResult::Allow { enable_srtp } => assert!(enable_srtp),
            _ => panic!("Expected Allow"),
        }
    }

    #[test]
    fn test_enforce_srtp_preferred_without_crypto() {
        let config = TrunkSrtpConfig {
            mode: SrtpMode::Preferred,
            allowed_suites: Vec::new(),
        };
        let result = enforce_srtp_policy(&config, &make_sdp_without_crypto());
        match result {
            SrtpPolicyResult::Allow { enable_srtp } => assert!(!enable_srtp),
            _ => panic!("Expected Allow"),
        }
    }

    #[test]
    fn test_enforce_srtp_required_with_crypto() {
        let config = TrunkSrtpConfig {
            mode: SrtpMode::Required,
            allowed_suites: Vec::new(),
        };
        let result = enforce_srtp_policy(&config, &make_sdp_with_crypto());
        match result {
            SrtpPolicyResult::Allow { enable_srtp } => assert!(enable_srtp),
            _ => panic!("Expected Allow"),
        }
    }

    #[test]
    fn test_enforce_srtp_required_without_crypto() {
        let config = TrunkSrtpConfig {
            mode: SrtpMode::Required,
            allowed_suites: Vec::new(),
        };
        let result = enforce_srtp_policy(&config, &make_sdp_without_crypto());
        match result {
            SrtpPolicyResult::Reject { reason } => {
                assert!(reason.contains("SRTP required"));
            }
            _ => panic!("Expected Reject"),
        }
    }
}
