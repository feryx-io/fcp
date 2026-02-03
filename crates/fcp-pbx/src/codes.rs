//! Client Matter Code (CMC) and Forced Authorization Code (FAC) validation.
//!
//! Determines whether a destination requires code entry, validates entered
//! codes, and reports the digit count for DTMF collection.

use crate::config::{CmcConfig, FacConfig};

/// What authorization is required before placing a call to a destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationRequirement {
    /// No authorization codes required.
    None,
    /// A Client Matter Code is required (billing/tracking).
    CmcRequired { digits: u32 },
    /// A Forced Authorization Code is required (access control).
    FacRequired { digits: u32 },
    /// Both FAC and CMC are required (FAC first, then CMC).
    Both { fac_digits: u32, cmc_digits: u32 },
}

/// Check whether a destination requires CMC, FAC, or both.
pub fn check_required(
    destination: &str,
    cmc_config: &CmcConfig,
    fac_config: &FacConfig,
) -> AuthorizationRequirement {
    let needs_cmc = cmc_config.enabled
        && cmc_config
            .required_for
            .iter()
            .any(|pattern| matches_pattern(pattern, destination));

    let needs_fac = fac_config.enabled
        && fac_config
            .required_for
            .iter()
            .any(|pattern| matches_pattern(pattern, destination));

    match (needs_fac, needs_cmc) {
        (true, true) => AuthorizationRequirement::Both {
            fac_digits: fac_config.digits,
            cmc_digits: cmc_config.digits,
        },
        (true, false) => AuthorizationRequirement::FacRequired {
            digits: fac_config.digits,
        },
        (false, true) => AuthorizationRequirement::CmcRequired {
            digits: cmc_config.digits,
        },
        (false, false) => AuthorizationRequirement::None,
    }
}

/// Validate a Forced Authorization Code against the configured allowlist.
///
/// Returns `true` if the code is in `valid_codes`, or if `valid_codes` is empty
/// (any numeric code accepted).
pub fn validate_fac(code: &str, config: &FacConfig) -> bool {
    if config.valid_codes.is_empty() {
        // Accept any numeric code of the right length
        code.len() == config.digits as usize && code.chars().all(|c| c.is_ascii_digit())
    } else {
        config.valid_codes.iter().any(|valid| valid == code)
    }
}

/// Validate a Client Matter Code.
///
/// Returns `true` if the code is in `valid_codes`, or if `valid_codes` is empty
/// (any numeric code of the right length accepted).
pub fn validate_cmc(code: &str, config: &CmcConfig) -> bool {
    if config.valid_codes.is_empty() {
        code.len() == config.digits as usize && code.chars().all(|c| c.is_ascii_digit())
    } else {
        config.valid_codes.iter().any(|valid| valid == code)
    }
}

/// Match a destination against a simple pattern.
///
/// Pattern syntax (inspired by Asterisk dial-plan):
/// - `_` at the start marks the remaining string as a digit pattern:
///   - `X` = any digit (0-9)
///   - `N` = any digit 2-9
///   - `0`-`9` = literal digit
///   - Any other char = literal match
/// - A trailing `*` is a wildcard (matches any remaining characters).
/// - Without `_` prefix: exact string match.
fn matches_pattern(pattern: &str, destination: &str) -> bool {
    // Extract the number portion from a SIP URI if present
    let number = extract_number(destination);

    if let Some(digit_pattern) = pattern.strip_prefix('_') {
        matches_digit_pattern(digit_pattern, number)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        number.starts_with(prefix)
    } else {
        number == pattern
    }
}

/// Extract the phone number from a SIP URI or raw string.
///
/// "sip:+15551234567@host" → "+15551234567"
/// "+15551234567" → "+15551234567"
/// "01144..." → "01144..."
fn extract_number(destination: &str) -> &str {
    let without_scheme = destination
        .strip_prefix("sip:")
        .or_else(|| destination.strip_prefix("sips:"))
        .unwrap_or(destination);
    without_scheme.split('@').next().unwrap_or(without_scheme)
}

/// Match a number against a digit pattern (after `_` prefix is stripped).
fn matches_digit_pattern(pattern: &str, number: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    let mut number_chars = number.chars();

    loop {
        match (pattern_chars.next(), number_chars.next()) {
            (Some('X'), Some(d)) => {
                if !d.is_ascii_digit() {
                    return false;
                }
            }
            (Some('N'), Some(d)) => {
                if !d.is_ascii_digit() || d < '2' {
                    return false;
                }
            }
            (Some(p), Some(d)) => {
                if p != d {
                    return false;
                }
            }
            (None, None) => return true,
            (None, Some(_)) => return false,
            (Some(_), None) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nanp_pattern() {
        // _1NXXNXXXXXX matches 11-digit NANP numbers starting with 1
        assert!(matches_pattern("_1NXXNXXXXXX", "12125551234"));
        assert!(!matches_pattern("_1NXXNXXXXXX", "+12125551234")); // +prefix = 12 chars, pattern expects 11
        assert!(!matches_pattern("_1NXXNXXXXXX", "2125551234")); // 10 digits
        assert!(!matches_pattern("_1NXXNXXXXXX", "11005551234")); // N position has 0
    }

    #[test]
    fn test_wildcard_suffix() {
        assert!(matches_pattern("011*", "01144207946"));
        assert!(matches_pattern("011*", "011"));
        assert!(!matches_pattern("011*", "001"));
    }

    #[test]
    fn test_exact_match() {
        assert!(matches_pattern("911", "911"));
        assert!(!matches_pattern("911", "9111"));
    }

    #[test]
    fn test_sip_uri_extraction() {
        assert!(matches_pattern("011*", "sip:01144207946@gateway.example.com"));
    }

    #[test]
    fn test_check_required_none() {
        let cmc = CmcConfig::default();
        let fac = FacConfig::default();
        assert_eq!(
            check_required("12125551234", &cmc, &fac),
            AuthorizationRequirement::None
        );
    }

    #[test]
    fn test_check_required_cmc_only() {
        let cmc = CmcConfig {
            enabled: true,
            required_for: vec!["_1NXXNXXXXXX".to_string()],
            valid_codes: vec![],
            digits: 6,
        };
        let fac = FacConfig::default();
        assert_eq!(
            check_required("12125551234", &cmc, &fac),
            AuthorizationRequirement::CmcRequired { digits: 6 }
        );
    }

    #[test]
    fn test_check_required_fac_only() {
        let cmc = CmcConfig::default();
        let fac = FacConfig {
            enabled: true,
            required_for: vec!["011*".to_string()],
            valid_codes: vec!["1234".to_string()],
            digits: 4,
        };
        assert_eq!(
            check_required("01144207946", &cmc, &fac),
            AuthorizationRequirement::FacRequired { digits: 4 }
        );
    }

    #[test]
    fn test_check_required_both() {
        let cmc = CmcConfig {
            enabled: true,
            required_for: vec!["011*".to_string()],
            valid_codes: vec![],
            digits: 6,
        };
        let fac = FacConfig {
            enabled: true,
            required_for: vec!["011*".to_string()],
            valid_codes: vec!["1234".to_string()],
            digits: 4,
        };
        assert_eq!(
            check_required("01144207946", &cmc, &fac),
            AuthorizationRequirement::Both {
                fac_digits: 4,
                cmc_digits: 6
            }
        );
    }

    #[test]
    fn test_validate_fac_valid() {
        let cfg = FacConfig {
            enabled: true,
            required_for: vec![],
            valid_codes: vec!["1234".to_string(), "5678".to_string()],
            digits: 4,
        };
        assert!(validate_fac("1234", &cfg));
        assert!(validate_fac("5678", &cfg));
        assert!(!validate_fac("0000", &cfg));
        assert!(!validate_fac("123", &cfg));
    }

    #[test]
    fn test_validate_fac_any_numeric() {
        let cfg = FacConfig {
            enabled: true,
            required_for: vec![],
            valid_codes: vec![],
            digits: 4,
        };
        assert!(validate_fac("0000", &cfg));
        assert!(validate_fac("9999", &cfg));
        assert!(!validate_fac("12345", &cfg)); // too long
        assert!(!validate_fac("123", &cfg)); // too short
        assert!(!validate_fac("abcd", &cfg)); // not numeric
    }

    #[test]
    fn test_validate_cmc() {
        let cfg = CmcConfig {
            enabled: true,
            required_for: vec![],
            valid_codes: vec![],
            digits: 6,
        };
        assert!(validate_cmc("123456", &cfg));
        assert!(!validate_cmc("12345", &cfg));
        assert!(!validate_cmc("1234567", &cfg));
    }
}
