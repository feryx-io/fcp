//! Intercom parameters and auto-answer header generation.
//!
//! The handler adds these headers to the outbound INVITE so that the target
//! phone auto-answers the call.

/// Intercom mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntercomMode {
    /// One-way: target phone auto-answers but only listens (sendonly SDP).
    OneWay,
    /// Two-way: target phone auto-answers with full duplex audio.
    TwoWay,
}

/// Parameters for an intercom call that the handler uses to construct
/// the outbound INVITE.
#[derive(Debug, Clone)]
pub struct IntercomParams {
    /// Target extension to intercom.
    pub target_extension: String,

    /// One-way or two-way mode.
    pub mode: IntercomMode,

    /// Extra SIP headers to add to the outbound INVITE.
    /// Each entry is (header-name, header-value).
    pub extra_headers: Vec<(String, String)>,
}

/// Build intercom parameters with auto-answer headers.
///
/// Standard auto-answer headers supported by most IP phones:
/// - `Alert-Info: <...>;info=alert-autoanswer;delay=0`
/// - `Call-Info: <sip:>;answer-after=0`
///
/// For one-way mode, an additional header signals the handler to modify
/// the SDP direction to `sendonly` on the intercom leg.
pub fn build_intercom_params(target_extension: &str, mode: IntercomMode) -> IntercomParams {
    let mut extra_headers = vec![
        (
            "Alert-Info".to_string(),
            "<http://127.0.0.1/>;info=alert-autoanswer;delay=0".to_string(),
        ),
        (
            "Call-Info".to_string(),
            "<sip:>;answer-after=0".to_string(),
        ),
    ];

    if mode == IntercomMode::OneWay {
        extra_headers.push(("X-FCP-Intercom-Mode".to_string(), "oneway".to_string()));
    }

    IntercomParams {
        target_extension: target_extension.to_string(),
        mode,
        extra_headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twoway_headers() {
        let params = build_intercom_params("1001", IntercomMode::TwoWay);
        assert_eq!(params.target_extension, "1001");
        assert_eq!(params.mode, IntercomMode::TwoWay);
        assert_eq!(params.extra_headers.len(), 2);
        assert_eq!(params.extra_headers[0].0, "Alert-Info");
        assert_eq!(params.extra_headers[1].0, "Call-Info");
    }

    #[test]
    fn test_oneway_headers() {
        let params = build_intercom_params("1001", IntercomMode::OneWay);
        assert_eq!(params.extra_headers.len(), 3);
        assert_eq!(params.extra_headers[2].0, "X-FCP-Intercom-Mode");
        assert_eq!(params.extra_headers[2].1, "oneway");
    }
}
